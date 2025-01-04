use cache::{compute_file_metadata, save_cache};
use env_logger;
use gray_matter::engine::YAML;
use gray_matter::Matter;
use log::{debug, error, info};
use minify_html::{minify, Cfg};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use tera::{Context, Tera};
use walkdir::WalkDir;

mod cache;
mod parser;
mod path;
mod resources;

// Match raw frontmatter input directly before being further parsed
// into more appropriate page type (separate frontmatter and metadata)
#[derive(Debug, Deserialize)]
struct Frontmatter {
    title: Option<String>,
    date: Option<String>,
    tags: Option<Vec<String>>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Config {
    metadata: SiteMetadata,
    paths: Paths,
    build: Build,
}

#[derive(Debug, Deserialize)]
struct SiteMetadata {
    base_url: String,
    author: String,
    description: String,
}

#[derive(Debug, Deserialize)]
struct Paths {
    content_dir: PathBuf,
    template_dir: PathBuf,
    output_dir: PathBuf,
    static_dir: PathBuf,
}

// TODO: Implement sitemap generation feature
#[derive(Debug, Deserialize)]
struct Build {
    minify_html: bool,
    generate_sitemap: bool,
    cache: bool,
}

// metadata for a page/post
#[derive(Debug, Serialize)]
enum PageType {
    Index,
    Page, // regular information page
    Post,
}

// NOTE: Treat a page and a post the exact same (just that a post an optional will have a date/tag as well)
// A page one of multiple types, represented as one of:
// - Post
// - Information (index, about, etc.)
#[derive(Debug, Serialize)]
struct Page {
    page_type: PageType,
    title: Option<String>,
    url: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
    date: Option<String>,
}

// // metadata for a blog post
// #[derive(Debug, Serialize)]
// struct Post {
//     title: String,
//     url: String,
//     description: String,
// }

// Reading YAML configuration files
fn load_yaml_config(file_path: &PathBuf) -> Result<Config, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(file_path)?;
    let config: Config = serde_yaml::from_str(&content)?;
    Ok(config)
}

/// Reconcile all paths in the configuration to absolute paths
fn reconcile_paths(base_path: &Path, config: Paths) -> Paths {
    Paths {
        content_dir: base_path.join(config.content_dir),
        template_dir: base_path.join(config.template_dir),
        output_dir: base_path.join(config.output_dir),
        static_dir: base_path.join(config.static_dir),
    }
}

// Build styling with Tailwind
// Assumes Tailwind exists globally on the system (npm install -g)
//
// NOTE: side effect of spawning a child process to execute tailwind
fn build_tailwind(build_path: &PathBuf) -> std::io::Result<()> {
    let output_path = build_path.join("tailwind.css");

    let status = Command::new("npx")
        .args([
            "tailwindcss",
            "-i",
            "input.css", // input css
            "-o",
            output_path.to_str().unwrap(), // output css
            "--minify",                    // minify the output
        ])
        .current_dir(build_path.join("../templates"))
        .status()?;

    if status.success() {
        info!("TailwindCSS build successful: {:?}", output_path);
        Ok(())
    } else {
        error!("TailwindCSS build failed: {:?}", output_path);
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Failed to build TailwindCSS",
        ))
    }
}

// Retrieve the folder name of a file (as matched to an HTML template)
fn get_template_name(path_to_file: &Path) -> &OsStr {
    // Retrieve the parent folder name to the file path
    let folder_path = path_to_file.parent().unwrap();
    let folder_name = folder_path.file_name().unwrap(); // returns the directory name

    // In the case that the folder name is a page, the template's HTML file will match the name of the markdown file
    // Otherwise, the folder name would match as the specific template (as multiple pages follow the same template)
    folder_name
}

fn extract_page_info(frontmatter: Frontmatter) -> Page {
    let page = Page {
        page_type: PageType::Page, // TODO: add post type based on directory name. temp stub
        title: frontmatter.title,
        url: Some(String::from("test url")),
        description: frontmatter.description,
        tags: frontmatter.tags,
        date: frontmatter.date,
    };

    page
}

fn main() -> std::io::Result<()> {
    env_logger::init();

    info!("Retrieving config file");
    let config_path = path::resolve_base_path("CONFIG_PATH", "../config.yml");
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    debug!("{:?}", config_path);
    let mut config = load_yaml_config(&config_path).unwrap();

    config.paths = reconcile_paths(config_dir, config.paths);
    debug!("{:?}", config);

    let template_dir = &config.paths.template_dir;
    let content_dir = &config.paths.content_dir;
    let output_dir = &config.paths.output_dir;
    let static_dir = &config.paths.static_dir;

    info!("Retrieving cache JSON file");
    // The cache will exist within the bin folder
    let cache_path = output_dir.join("cache.json");
    let mut cache = cache::load_cache(&cache_path).unwrap_or_default();

    let template_dir = &config.paths.template_dir;
    let content_dir = &config.paths.content_dir;
    let output_dir = &config.paths.output_dir;
    let static_dir = &config.paths.static_dir;

    // HANDLE STATIC CONTENT (from resources.rs folder)
    // copy the static directory into the build folder.
    // These files do not require any extra processing by the SSG
    // But I added steps to optimize file sizes/image formats
    resources::optimize_and_copy_static_folder(
        Path::new(static_dir),
        Path::new(output_dir).join("static").as_path(),
        &output_dir.join("static-cache.json"),
    )?;

    // Initialize Tera for templating with the HTML files
    let tera =
        Tera::new(format!("{}/**/*.html", template_dir.to_string_lossy().to_string()).as_str());

    // Create output directory
    fs::create_dir_all(output_dir)?;

    for entry in WalkDir::new(content_dir).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let path_buf = path.to_path_buf();

            // Check if unmodified based on hash & modify metadata in cache
            if config.build.cache {
                if cache::has_file_changed(&path_buf, &cache)? {
                    info!("Rebuilding: {:?}", path);
                    // update the cache
                    cache
                        .file_data
                        .insert(path_buf.clone(), compute_file_metadata(&path_buf)?);
                } else {
                    info!("Skipping unchanged file: {:?}", path_buf);
                    continue;
                }
            } else {
                info!(
                    "File {:?} not cached. Cache on this build is disabled (check config.yml file)",
                    path_buf
                );
            }

            // Read markdown file
            let markdown = fs::read_to_string(path)?;

            // Retrieve the Markdown frontmatter & parse
            let matter = Matter::<YAML>::new();
            let parsed_frontmatter = matter.parse(&markdown);

            let page = if let Some(front) = parsed_frontmatter.data {
                let frontmatter: Frontmatter = front.deserialize().unwrap();
                Some(extract_page_info(frontmatter))
            } else {
                None
            };

            let content = parsed_frontmatter.content;

            // Convert Markdown to HTML
            let html_output =
                parser::parse_markdown_with_tailwind(&content, &tera.as_ref().unwrap());

            // Render with template
            let mut context = Context::new();
            if let Some(page) = page {
                context.insert("title", &page.title);
            } else {
                context.insert("title", path.file_stem().unwrap().to_str().unwrap());
            }
            context.insert("content", &html_output);
            context.insert("author", &config.metadata.author);
            context.insert("description", &config.metadata.description);

            // STUB. Remove hardcode of the post names
            // TODO: remove hardcode and grab actual page/post info from content folder
            let pages: Vec<Page> = vec![
                // Page {
                //     title: String::from("About"),
                //     url: String::from("url"),
                //     description: String::from("description test"),
                // },
                // Page {
                //     title: String::from("Experience"),
                //     url: String::from("url"),
                //     description: String::from("description test"),
                // },
                // Page {
                //     title: String::from("Test"),
                //     url: String::from("url"),
                //     description: String::from("description test"),
                // },
            ];

            let posts: Vec<Page> = vec![
                // Post {
                //     title: String::from("Test Post 1"),
                //     url: String::from("url"),
                //     description: String::from("Description 1"),
                // },
                // Post {
                //     title: String::from("Test Post 2"),
                //     url: String::from("url"),
                //     description: String::from("Description 2"),
                // },
            ];

            context.insert("pages", &pages);
            context.insert("posts", &posts);

            let html_template_file = get_template_name(path);
            let html_file_name = format!("{}.html", html_template_file.to_string_lossy());
            let rendered = tera.as_ref().unwrap().render(&html_file_name, &context);

            // CSS styling output
            let _ = build_tailwind(output_dir);

            // Create the output file name
            let output_path = Path::new(&config.paths.output_dir)
                .join(path.file_stem().unwrap().to_str().unwrap().to_string() + ".html");

            // If desired, run the minify utility before outputting the final build
            // files. This is defined within the config YAML file
            if config.build.minify_html {
                let cfg = Cfg {
                    minify_js: true,
                    minify_css: false, // Tailwind already out
                    ..Default::default()
                };
                let minified = minify(rendered.unwrap().as_bytes(), &cfg);
                fs::write(output_path, minified)?;
            } else {
                // Write output file
                fs::write(output_path, rendered.unwrap())?;
            }
        }
    }

    let _ = save_cache(&cache, &cache_path);

    info!("Static site generated in 'output/' directory!");
    Ok(())
}
