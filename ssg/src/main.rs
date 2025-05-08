use cache::CacheContext;
use env_logger;
use gray_matter::engine::YAML;
use gray_matter::Matter;
use log::{debug, error, info};
use minify_html::{minify, Cfg};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
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

// Retrieve configuration information from the configuration YAML file
#[derive(Debug, Deserialize)]
struct Config {
    metadata: SiteMetadata,
    paths: Paths,
    build: Build,
}

// Metadata retrieved from the configuration YAML file
#[derive(Debug, Deserialize)]
struct SiteMetadata {
    base_url: String,
    author: String,
    description: String,
}

// Paths for content, template, output build, and the static resourees. Specified as the path
// relative to the configuration YAML file. NOT the Rust project directories
#[derive(Debug, Deserialize)]
struct Paths {
    content_dir: PathBuf,
    template_dir: PathBuf,
    output_dir: PathBuf,
    static_dir: PathBuf,
}

// Build configuration specifications from configuration YAML.
// TODO: Implement sitemap generation feature
#[derive(Debug, Deserialize)]
struct Build {
    minify_html: bool,
    generate_sitemap: bool,
    cache: bool,
}

// A page is one of: Index (for the main page), a Page ("supporting" information page), a Post
// (blog post)
// Else: label as "Unknown" to warn the user that this has yet to be integrated
#[derive(Debug, Serialize, Clone)]
enum PageType {
    Index,
    Page, // regular information page
    Post,
    Unknown,
}

// NOTE: Treat a page and a post the exact same (just that a post an optional will have a date/tag as well)
// A page one of multiple types, represented as one of:
// - Post
// - Information (index, about, etc.)
#[derive(Debug, Serialize)]
struct Page {
    page_type: PageType,
    name: String,
    title: Option<String>,
    url: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
    date: Option<String>,
    content: String,
}

#[derive(Debug)]
struct Site {
    configuration: Config,
    index: Option<Page>,
    pages: Vec<Page>,
    posts: Vec<Page>,
}

impl Site {
    fn new(config: Config) -> Self {
        Site {
            configuration: config,
            index: None,
            pages: Vec::new(),
            posts: Vec::new(),
        }
    }

    fn add_page(&mut self, page: Page, page_type: PageType) {
        match page_type {
            PageType::Index => {
                self.index = Some(page);
            }
            PageType::Page => {
                self.pages.push(page);
            }
            PageType::Post => {
                self.posts.push(page);
            }
            PageType::Unknown => {
                // do nothing
            }
        }
    }

    fn get_template_name(page: &Page) -> String {
        match page.page_type {
            PageType::Index => String::from("index"),
            PageType::Page => String::from("page"),
            PageType::Post => String::from("post"),
            PageType::Unknown => String::from(""),
        }
    }

    fn generate_page(&self, page: &Page, tera: &Tera) -> Result<(), Box<dyn std::error::Error>> {
        let html_output = parser::parse_markdown_with_tailwind(&page.content, tera);

        let mut context = Context::new();
        context.insert("title", &page.title);
        context.insert("date", &page.date);
        context.insert("content", &html_output);
        context.insert("author", &self.configuration.metadata.author);
        context.insert("description", &self.configuration.metadata.description);
        context.insert("pages", &self.pages);
        context.insert("posts", &self.posts);

        let output_filename = format!("{}.html", page.name);
        let html_template_file = Site::get_template_name(page);
        let rendered = tera.render(&format!("{}.html", html_template_file), &context)?;

        let output_path = Path::new(&self.configuration.paths.output_dir).join(output_filename);
        if self.configuration.build.minify_html {
            let minified = minify(
                rendered.as_bytes(),
                &Cfg {
                    minify_js: true,
                    minify_css: false,
                    ..Default::default()
                },
            );
            fs::write(output_path, minified)?;
        } else {
            fs::write(output_path, rendered)?;
        }

        Ok(())
    }
}

// Reading YAML configuration files
fn load_yaml_config(file_path: &PathBuf) -> Result<Config, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(file_path)?;
    let config: Config = serde_yaml::from_str(&content)?;
    Ok(config)
}

// Resolves all directory paths relative to the configuration YAML file to
// to change to absbolute paths in the project
fn reconcile_configuration_directory_paths(base_path: &Path, config: Paths) -> Paths {
    Paths {
        content_dir: base_path.join(config.content_dir),
        template_dir: base_path.join(config.template_dir),
        output_dir: base_path.join(config.output_dir),
        static_dir: base_path.join(config.static_dir),
    }
}

// Grabs the configuration file relative to the location in the CONFIG_PATH environment variable
fn retrieve_configuration() -> Config {
    info!("Retrieving config file");

    let config_path = path::resolve_environment_variable_path("CONFIG_PATH", "../config.yml");
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    debug!("{:?}", config_path);
    let mut config = load_yaml_config(&config_path).unwrap();

    config.paths = reconcile_configuration_directory_paths(config_dir, config.paths);
    debug!("{:?}", config);

    config
}

fn retrieve_cache(config: &Config) -> CacheContext {
    info!("Retrieving cache JSON file");
    // The cache will exist within the bin folder
    let cache_path = config.paths.output_dir.join("cache.json");
    CacheContext::load_or_default(cache_path).unwrap()
}

// Build styling with Tailwind
// Assumes Tailwind exists globally on the system (npm install -g)
//
// NOTE: side effect of spawning a child process to execute tailwind
// Retrieve the folder name of a file (as matched to an HTML template)
fn build_tailwind(site: &Site) -> std::io::Result<()> {
    let output_path = site
        .configuration
        .paths
        .output_dir
        .join("static/styles/tailwind.css");
    let working_dir = site.configuration.paths.template_dir.clone();

    info!("Tailwind build starting");
    info!("Output path: {:?}", output_path);
    info!("Working directory: {:?}", working_dir);

    // Check path existence
    if !working_dir.exists() {
        error!("Working directory does not exist: {:?}", working_dir);
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Directory not found: {:?}", working_dir),
        ));
    }

    if which::which("npx").is_err() {
        error!("`npx` not found in PATH!");
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "npx not found in PATH",
        ));
    }

    // Log command being run
    info!(
        "Running command: npx tailwindcss -i input.css -o {} --minify",
        output_path.to_str().unwrap_or("<invalid path>")
    );

    let output = Command::new("npx")
        .args([
            "tailwindcss",
            "-i",
            "input.css",
            "-o",
            output_path.to_str().unwrap(),
            "--minify",
        ])
        .current_dir(&working_dir)
        .output();

    match output {
        Ok(output) => {
            info!("Command completed with status: {}", output.status);
            if !output.status.success() {
                error!("Tailwind build failed");
                error!("stdout:\n{}", String::from_utf8_lossy(&output.stdout));
                error!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Tailwind build failed",
                ));
            }
        }
        Err(e) => {
            error!("Failed to spawn command: {}", e);
            return Err(e);
        }
    }

    info!("Tailwind build succeeded");
    Ok(())
}

fn get_template_name(path_to_file: &Path) -> &OsStr {
    // Retrieve the parent folder name to the file path
    let folder_path = path_to_file.parent().unwrap();
    let folder_name = folder_path.file_name().unwrap(); // returns the directory name

    // In the case that the folder name is a page, the template's HTML file will match the name of the markdown file
    // Otherwise, the folder name would match as the specific template (as multiple pages follow the same template)
    folder_name
}

fn extract_page_info(
    base_url: String,
    path: &Path,
    frontmatter: Frontmatter,
    content: String,
    page_type: PageType,
) -> Page {
    let name = path.file_stem().unwrap().to_str().unwrap().to_string();

    let page = Page {
        page_type,
        name: name.clone(),
        title: frontmatter.title,
        url: Some(base_url + "/" + name.as_str() + ".html"),
        description: frontmatter.description,
        tags: frontmatter.tags,
        date: frontmatter.date,
        content,
    };

    page
}

fn main() -> std::io::Result<()> {
    // Initialize the logger (which uses an environment variable to correspondingly toggle)
    env_logger::init();

    let config = retrieve_configuration();
    let mut cache_context = retrieve_cache(&config);

    let mut site = Site::new(config);
    info!(
        "Start generation for site with base URL: {:?}",
        site.configuration.metadata.base_url
    );
    info!(
        "Build configuration: \n Minify HTML: {:?} \n Sitemap generation: {:?}",
        site.configuration.build.minify_html, site.configuration.build.generate_sitemap
    );

    let template_dir = &site.configuration.paths.template_dir;
    let content_dir = &site.configuration.paths.content_dir;
    let output_dir = &site.configuration.paths.output_dir;
    let static_dir = &site.configuration.paths.static_dir;

    let template_filepath = format!(
        "{}/**/*.html",
        template_dir
            .to_str()
            .expect("Template directory must be a UTF-8")
    );
    let tera_result = Tera::new(&template_filepath);
    let tera = tera_result.unwrap();

    // Handles static resources (images, etc)
    // copy the static directory into the build folder.
    // These files do not require any extra processing by the SSG
    // but the following function runs an optimization for image files
    // Saves static files within a nested /static folder within the output directory
    resources::optimize_and_copy_static_folder(
        Path::new(static_dir),
        Path::new(output_dir).join("static").as_path(),
        &output_dir.join("static-cache.json"),
    )?;

    // Create output directory for the build results
    fs::create_dir_all(output_dir)?;

    // Pass 1: Create the Site struct representing the website based on recursively walking through
    // the directories
    // Note: files are added to the Site struct in descending order by filename (from the folder in which they are found)
    let mut files: Vec<walkdir::DirEntry> = WalkDir::new(content_dir).into_iter().filter_map(Result::ok).collect();
    files.sort_by(|a, b| b.file_name().cmp(a.file_name()));

    for entry in files {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let path_buf = path.to_path_buf();

            // Check if unmodified based on hash & modify metadata in cache
            if site.configuration.build.cache {
                if !cache_context.update_file_if_changed(&path_buf)? {
                    // If the file was already in the cache, move onto the next file
                    // without rebuilding the page
                    continue;
                } else {
                    info!("File {:?} was changed. Rebuilding", path_buf);
                }
            } else {
                info!(
                    "File {:?} was not cached. Cache on this build is disabled (check config.yml file)",
                    path_buf
                );
            }

            // Read markdown file
            let markdown = fs::read_to_string(path)?;

            // Retrieve the Markdown frontmatter & parse
            let matter = Matter::<YAML>::new();
            let parsed_frontmatter = matter.parse(&markdown);

            let html_template_file = get_template_name(path);
            let page_type = match html_template_file.to_str().unwrap() {
                "index" => PageType::Index,
                "page" => PageType::Page,
                "post" => PageType::Post,
                _ => PageType::Unknown,
            };
            let page = if let Some(front) = parsed_frontmatter.data {
                let frontmatter: Frontmatter = front.deserialize().unwrap();
                Some(extract_page_info(
                    site.configuration.metadata.base_url.clone(),
                    path,
                    frontmatter,
                    parsed_frontmatter.content,
                    page_type.clone(),
                ))
            } else {
                None
            };

            if let Some(page) = page {
                // page metadata exists, add to the site data structure
                Site::add_page(&mut site, page, page_type);
            }
        }
    }

    // Pass 2: Generate the HTML for each page in the site
    // 1. Generate the index page
    // 2. Generate the other pages
    // 3. Generate the blog posts
    // Initialize Tera for templating with the HTML files

    if let Some(index) = site.index.as_ref() {
        if let Err(e) = site.generate_page(index, &tera) {
            error!("Failed to generate index page '{}': {}", index.name, e);
        }
    } else {
        error!("No index page found in site data");
    }

    for page in &site.pages {
        if let Err(e) = site.generate_page(page, &tera) {
            error!("Failed to generate page '{}': {}", page.name, e);
        }
    }

    for post in &site.posts {
        if let Err(e) = site.generate_page(post, &tera) {
            error!("Failed to generate post '{}': {}", post.name, e);
        }
    }
    let _ = build_tailwind(&site);
    info!("Static site generated in 'output/' directory!");
    debug!("Site generated: {:?}", site);
    Ok(())
}
