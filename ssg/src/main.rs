use cache::{BuildCache, Fingerprint};
use chrono::Datelike;
use gray_matter::engine::YAML;
use gray_matter::Matter;
use log::{debug, error, info, warn};
use minify_html::{minify, Cfg};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use tera::{Context, Tera};
use walkdir::WalkDir;

mod cache;
mod parser;
mod path;
mod resources;
#[cfg(test)]
mod test_util;

type BoxResult<T> = Result<T, Box<dyn Error>>;

// Name of the folder (within the output directory) that static resources are written to
const STATIC_OUTPUT_DIR: &str = "static";

// Name of the cache file, stored next to the configuration YAML file (outside of the output
// directory so it is never deployed)
const CACHE_FILE_NAME: &str = ".ssg-cache.json";

const USAGE: &str = "Usage: ssg [--force] [--clean] [--no-css]

Options:
  --force    Rebuild every file, ignoring the build cache
  --clean    Delete the output directory and build cache before building
  --no-css   Skip the Tailwind CSS build (no Node.js needed)
  -h, --help Print this message

Environment:
  CONFIG_PATH  Path to the configuration YAML file (default: ../config.yml)
  RUST_LOG     Log level (error, warn, info, debug; default: info)";

// Match raw frontmatter input directly before being further parsed
// into more appropriate page type (separate frontmatter and metadata)
#[derive(Debug, Deserialize)]
struct Frontmatter {
    title: Option<String>,
    date: Option<String>,
    tags: Option<Vec<String>>,
    description: Option<String>,
    // Position of a page in listings like the navigation (lowest first). Pages without one are
    // listed after the ordered ones, alphabetically
    order: Option<i64>,
    // Output name (<slug>.html), defaults to the markdown file name
    slug: Option<String>,
    // Previous names of the page. Each gets a small page redirecting to the current URL so old
    // links keep working
    aliases: Option<Vec<String>>,
}

// Retrieve configuration information from the configuration YAML file
#[derive(Debug, Deserialize)]
struct Config {
    metadata: SiteMetadata,
    paths: Paths,
    #[serde(default)]
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
#[serde(default)]
struct Build {
    minify_html: bool,
    generate_sitemap: bool,
    cache: bool,
}

impl Default for Build {
    fn default() -> Self {
        Build {
            minify_html: true,
            generate_sitemap: false,
            cache: true,
        }
    }
}

// Command line flags
#[derive(Debug, Default)]
struct Args {
    force: bool,
    clean: bool,
    no_css: bool,
}

// A page is one of: Index (for the main page), a Page ("supporting" information page), a Post
// (blog post)
// Else: label as "Unknown" to warn the user that this has yet to be integrated
#[derive(Debug, Serialize, Clone, Copy, PartialEq)]
enum PageType {
    Index,
    Page, // regular information page
    Post,
    Unknown,
}

impl PageType {
    // The folder name of a file matches the HTML template (as multiple pages follow the same
    // template)
    fn from_folder_name(name: &str) -> Self {
        match name {
            "index" => PageType::Index,
            "page" => PageType::Page,
            "post" => PageType::Post,
            _ => PageType::Unknown,
        }
    }

    fn template_name(self) -> Option<&'static str> {
        match self {
            PageType::Index => Some("index.html"),
            PageType::Page => Some("page.html"),
            PageType::Post => Some("post.html"),
            PageType::Unknown => None,
        }
    }
}

// NOTE: Treat a page and a post the exact same (just that a post an optional will have a date/tag as well)
// A page one of multiple types, represented as one of:
// - Post
// - Information (index, about, etc.)
//
// The summary is what other pages see of this page (navigation, recent post lists, ...)
#[derive(Debug, Serialize, Clone)]
struct PageSummary {
    page_type: PageType,
    name: String,
    order: Option<i64>,
    title: Option<String>,
    url: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
    date: Option<String>,
}

#[derive(Debug)]
struct Page {
    summary: PageSummary,
    aliases: Vec<String>,
    source: PathBuf,
    // Markdown content (without the frontmatter)
    content: String,
}

struct Site {
    configuration: Config,
    config_path: PathBuf,
    // Year of the build, available to templates (e.g. the footer) as `current_year`
    current_year: i32,
    index: Option<Page>,
    pages: Vec<Page>,
    posts: Vec<Page>,
}

impl Site {
    fn new(configuration: Config, config_path: PathBuf) -> Self {
        Site {
            configuration,
            config_path,
            current_year: chrono::Local::now().year(),
            index: None,
            pages: Vec::new(),
            posts: Vec::new(),
        }
    }

    fn add_page(&mut self, page: Page) {
        match page.summary.page_type {
            PageType::Index => {
                if let Some(previous) = &self.index {
                    warn!(
                        "Multiple index pages found, {:?} replaces {:?}",
                        page.source, previous.source
                    );
                }
                self.index = Some(page);
            }
            PageType::Page => self.pages.push(page),
            PageType::Post => self.posts.push(page),
            PageType::Unknown => {
                warn!(
                    "Skipping {:?}: its folder does not match a known page type (index, page, post)",
                    page.source
                );
            }
        }
    }

    fn all_pages(&self) -> impl Iterator<Item = &Page> {
        self.index.iter().chain(&self.pages).chain(&self.posts)
    }

    // Generates the HTML for a page. Returns true if the page was (re)written, false if it was
    // already up to date in the cache.
    fn generate_page(
        &self,
        page: &Page,
        tera: &Tera,
        cache: &mut BuildCache,
        build_fingerprint: &Fingerprint,
    ) -> BoxResult<bool> {
        let summary = &page.summary;
        let template = summary
            .page_type
            .template_name()
            .ok_or("page has no template")?;
        let html_output = parser::parse_markdown_with_tailwind(&page.content, tera)?;

        let pages: Vec<&PageSummary> = self.pages.iter().map(|p| &p.summary).collect();
        let posts: Vec<&PageSummary> = self.posts.iter().map(|p| &p.summary).collect();
        let metadata = &self.configuration.metadata;

        let mut context = Context::new();
        context.insert("title", &summary.title);
        context.insert("date", &summary.date);
        context.insert("content", &html_output);
        context.insert("author", &metadata.author);
        context.insert(
            "description",
            summary
                .description
                .as_ref()
                .unwrap_or(&metadata.description),
        );
        context.insert("pages", &pages);
        context.insert("posts", &posts);
        context.insert("tags", &summary.tags);
        // Part of the context, so the cache rebuilds every page when the year changes
        context.insert("current_year", &self.current_year);

        // Everything that affects the output: SSG binary, templates, config, and the full template
        // context (which includes this page's content and the listing of every other page)
        let output_key = format!("{}.html", summary.name);
        let fingerprint = build_fingerprint
            .clone()
            .add(template)
            .add(context.clone().into_json().to_string())
            .finish();
        if cache.is_fresh(&output_key, &fingerprint) {
            return Ok(false);
        }

        let result = (|| -> BoxResult<()> {
            let rendered = tera.render(template, &context)?;
            let output = if self.configuration.build.minify_html {
                minify(
                    rendered.as_bytes(),
                    &Cfg {
                        minify_js: true,
                        minify_css: false,
                        ..Default::default()
                    },
                )
            } else {
                rendered.into_bytes()
            };
            fs::write(
                self.configuration.paths.output_dir.join(&output_key),
                output,
            )?;
            Ok(())
        })();

        if result.is_err() {
            cache.invalidate(&output_key);
        }
        result.map(|()| true)
    }

    // Writes a redirect page for each of the page's aliases. Returns how many were (re)written.
    fn generate_aliases(
        &self,
        page: &Page,
        cache: &mut BuildCache,
        build_fingerprint: &Fingerprint,
    ) -> BoxResult<usize> {
        let target = page.summary.url.as_deref().ok_or("page has no URL")?;
        let base_url = self.configuration.metadata.base_url.trim_end_matches('/');
        let html = redirect_html(target, &format!("{base_url}{target}"));

        let mut written = 0;
        for alias in &page.aliases {
            let output_key = format!("{alias}.html");
            let fingerprint = build_fingerprint.clone().add("alias").add(&html).finish();
            if cache.is_fresh(&output_key, &fingerprint) {
                continue;
            }
            if let Err(e) = fs::write(self.configuration.paths.output_dir.join(&output_key), &html)
            {
                cache.invalidate(&output_key);
                return Err(e.into());
            }
            written += 1;
        }
        Ok(written)
    }
}

// A page that sends visitors (and search engines, via the canonical link) to `target`. GitHub
// Pages can't do server-side redirects, so this is done with a meta refresh.
fn redirect_html(target: &str, canonical: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en-US\"><head><meta charset=\"utf-8\">\
         <title>Redirecting</title><link rel=\"canonical\" href=\"{canonical}\">\
         <meta name=\"robots\" content=\"noindex\">\
         <meta http-equiv=\"refresh\" content=\"0; url={target}\"></head>\
         <body><a href=\"{target}\">Redirecting to {target}</a></body></html>"
    )
}

// Page names and aliases become file names in the output directory, so only allow simple names
fn validate_name(name: &str, path: &Path, field: &str) -> BoxResult<()> {
    let valid = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "Invalid {field} {name:?} in {:?}: use only letters, numbers, '-', '_' and '.'",
            path
        )
        .into())
    }
}

// Pages are listed by their `order` (lowest first), then pages without an order by name
fn sort_pages(pages: &mut [Page]) {
    pages.sort_by(|a, b| {
        let (a, b) = (&a.summary, &b.summary);
        a.order
            .is_none()
            .cmp(&b.order.is_none())
            .then(a.order.cmp(&b.order))
            .then_with(|| a.name.cmp(&b.name))
    });
}

// Formats an error together with its causes (Tera puts the useful details, like which variable
// is undefined, in the source errors)
fn error_chain(error: &dyn Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

// Returns None when help was requested
fn parse_args(raw: impl IntoIterator<Item = String>) -> Result<Option<Args>, String> {
    let mut args = Args::default();
    for arg in raw {
        match arg.as_str() {
            "--force" | "-f" => args.force = true,
            "--clean" => args.clean = true,
            "--no-css" => args.no_css = true,
            "--help" | "-h" => return Ok(None),
            other => return Err(format!("Unknown argument: {other}")),
        }
    }
    Ok(Some(args))
}

// Reading YAML configuration files
fn load_yaml_config(file_path: &Path) -> BoxResult<Config> {
    let content = fs::read_to_string(file_path)
        .map_err(|e| format!("Could not read config file {:?}: {}", file_path, e))?;
    let config: Config = serde_yaml::from_str(&content)
        .map_err(|e| format!("Invalid config file {:?}: {}", file_path, e))?;
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
fn retrieve_configuration() -> BoxResult<(Config, PathBuf)> {
    info!("Retrieving config file");

    let config_path = path::resolve_environment_variable_path("CONFIG_PATH", "../config.yml");
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    debug!("{:?}", config_path);
    let mut config = load_yaml_config(&config_path)?;

    config.paths = reconcile_configuration_directory_paths(config_dir, config.paths);
    debug!("{:?}", config);

    for (name, dir) in [
        ("content_dir", &config.paths.content_dir),
        ("template_dir", &config.paths.template_dir),
        ("static_dir", &config.paths.static_dir),
    ] {
        if !dir.is_dir() {
            return Err(format!("{name} {:?} does not exist", dir).into());
        }
    }

    Ok((config, config_path))
}

// Delete the output directory, refusing to do so if it contains any of the site's inputs
fn clean_output_dir(config: &Config, config_path: &Path) -> BoxResult<()> {
    let output_dir = &config.paths.output_dir;
    let Ok(output_dir) = output_dir.canonicalize() else {
        return Ok(()); // nothing to clean
    };

    let inputs = [
        config_path,
        &config.paths.content_dir,
        &config.paths.template_dir,
        &config.paths.static_dir,
    ];
    for input in inputs {
        if input.canonicalize()?.starts_with(&output_dir) {
            return Err(format!(
                "Refusing to clean output directory {:?} because it contains {:?}",
                output_dir, input
            )
            .into());
        }
    }

    info!("Removing output directory {:?}", output_dir);
    fs::remove_dir_all(&output_dir)?;
    Ok(())
}

// Hash of the running executable so any change to the SSG itself invalidates generated pages.
// Falls back to the crate version if the executable cannot be read.
fn executable_fingerprint() -> String {
    match std::env::current_exe().and_then(fs::read) {
        Ok(bytes) => blake3::hash(&bytes).to_hex().to_string(),
        Err(e) => {
            warn!("Could not hash the SSG executable ({e}), falling back to its version");
            env!("CARGO_PKG_VERSION").to_string()
        }
    }
}

// Hash every (non-hidden) file in a directory, including the file names
fn directory_fingerprint(dir: &Path) -> BoxResult<String> {
    let mut fingerprint = Fingerprint::new();
    let entries = WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));
    for entry in entries {
        let entry = entry?;
        if entry.file_type().is_file() {
            fingerprint
                .add(cache::output_key(entry.path().strip_prefix(dir)?))
                .add(fs::read(entry.path())?);
        }
    }
    Ok(fingerprint.finish())
}

// Build styling with Tailwind
// Assumes Tailwind exists globally on the system (npm install -g)
//
// NOTE: side effect of spawning a child process to execute tailwind
fn build_tailwind(site: &Site) -> BoxResult<()> {
    let output_path = site
        .configuration
        .paths
        .output_dir
        .join(STATIC_OUTPUT_DIR)
        .join("styles/tailwind.css");
    let working_dir = &site.configuration.paths.template_dir;

    info!("Tailwind build starting");
    debug!("Output path: {:?}", output_path);
    debug!("Working directory: {:?}", working_dir);

    let output = Command::new("npx")
        .arg("tailwindcss")
        .arg("-i")
        .arg("input.css")
        .arg("-o")
        .arg(&output_path)
        .arg("--minify")
        .current_dir(working_dir)
        .output()
        .map_err(|e| format!("Failed to run `npx tailwindcss` (is Node.js installed?): {e}"))?;

    if !output.status.success() {
        error!("stdout:\n{}", String::from_utf8_lossy(&output.stdout));
        error!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));
        return Err(format!("Tailwind build failed with status {}", output.status).into());
    }

    info!("Tailwind build succeeded");
    Ok(())
}

// Pass 1: Create the Site struct representing the website based on recursively walking through
// the directories
// Note: files are added to the Site struct in descending order by filename (from the folder in
// which they are found)
fn load_content(site: &mut Site) -> BoxResult<()> {
    let content_dir = &site.configuration.paths.content_dir;
    let mut files = Vec::new();
    for entry in WalkDir::new(content_dir) {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type().is_file() && path.extension().and_then(|e| e.to_str()) == Some("md") {
            files.push(entry.into_path());
        }
    }
    files.sort_by(|a, b| b.file_name().cmp(&a.file_name()).then_with(|| b.cmp(a)));

    let matter = Matter::<YAML>::new();
    let mut pages = Vec::new();

    for path in files {
        // Read markdown file
        let markdown =
            fs::read_to_string(&path).map_err(|e| format!("Could not read {:?}: {}", path, e))?;

        // Retrieve the Markdown frontmatter & parse
        let parsed = matter.parse(&markdown);
        let Some(data) = parsed.data else {
            warn!("Skipping {:?}: it has no frontmatter", path);
            continue;
        };
        let frontmatter: Frontmatter = data
            .deserialize()
            .map_err(|e| format!("Invalid frontmatter in {:?}: {}", path, e))?;

        let folder_name = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let name = match frontmatter.slug {
            Some(slug) => slug,
            None => path
                .file_stem()
                .and_then(|n| n.to_str())
                .ok_or_else(|| format!("File name {:?} is not valid UTF-8", path))?
                .to_string(),
        };
        validate_name(&name, &path, "page name")?;

        // Aliases may be written with or without the .html extension
        let aliases = frontmatter
            .aliases
            .unwrap_or_default()
            .into_iter()
            .map(|alias| alias.trim_end_matches(".html").to_string())
            .collect::<Vec<_>>();
        for alias in &aliases {
            validate_name(alias, &path, "alias")?;
        }

        pages.push(Page {
            summary: PageSummary {
                page_type: PageType::from_folder_name(folder_name),
                // Root-relative so links work on the deployed site and when served locally (the
                // site is served from the root of its domain, like the "/" home links)
                url: Some(format!("/{name}.html")),
                name,
                order: frontmatter.order,
                title: frontmatter.title,
                description: frontmatter.description,
                tags: frontmatter.tags,
                date: frontmatter.date,
            },
            aliases,
            source: path,
            content: parsed.content,
        });
    }

    for page in pages {
        site.add_page(page);
    }
    // Posts stay in descending file name order (newest first for date-prefixed names)
    sort_pages(&mut site.pages);

    // Every page and alias is written to <output>/<name>.html, so names must be unique
    let mut names = HashSet::new();
    for page in site.all_pages() {
        for name in std::iter::once(&page.summary.name).chain(&page.aliases) {
            if !names.insert(name) {
                return Err(format!(
                    "Multiple pages or aliases are named {:?} (each would be written to {}.html)",
                    name, name
                )
                .into());
            }
        }
    }

    Ok(())
}

fn run(args: Args) -> BoxResult<()> {
    let (config, config_path) = retrieve_configuration()?;
    let mut site = Site::new(config, config_path);
    let config = &site.configuration;

    info!(
        "Start generation for site with base URL: {:?}",
        config.metadata.base_url
    );
    info!(
        "Build configuration: \n Minify HTML: {:?} \n Sitemap generation: {:?} \n Cache: {:?}",
        config.build.minify_html, config.build.generate_sitemap, config.build.cache
    );
    if config.build.generate_sitemap {
        warn!("Sitemap generation is not implemented yet, ignoring generate_sitemap");
    }

    let config_dir = site.config_path.parent().unwrap_or_else(|| Path::new("."));
    let cache_path = config_dir.join(CACHE_FILE_NAME);
    if args.clean {
        clean_output_dir(config, &site.config_path)?;
        if cache_path.exists() {
            fs::remove_file(&cache_path)?;
        }
    }

    let output_dir = config.paths.output_dir.clone();
    // Create output directory for the build results
    fs::create_dir_all(&output_dir)?;

    let use_cache = config.build.cache && !args.force;
    if !use_cache {
        info!("Cache disabled for this build (--force or `build.cache: false`), rebuilding everything");
    }
    let mut cache = BuildCache::load(cache_path, output_dir.clone(), use_cache);

    // Handles static resources (images, etc)
    // copy the static directory into the build folder.
    // These files do not require any extra processing by the SSG
    // but the following function runs an optimization for image files
    // Saves static files within a nested /static folder within the output directory
    resources::optimize_and_copy_static_folder(
        &config.paths.static_dir,
        &output_dir,
        STATIC_OUTPUT_DIR,
        &mut cache,
    )?;

    // Initialize Tera for templating with the HTML files
    let template_dir = &config.paths.template_dir;
    let template_glob = format!(
        "{}/**/*.html",
        template_dir
            .to_str()
            .ok_or("Template directory must be valid UTF-8")?
    );
    let tera = Tera::new(&template_glob)?;

    let mut build_fingerprint = Fingerprint::new();
    build_fingerprint
        .add(executable_fingerprint())
        .add(directory_fingerprint(template_dir)?)
        .add(config.build.minify_html.to_string());

    load_content(&mut site)?;
    if site.index.is_none() {
        error!("No index page found in site data");
    }

    // Pass 2: Generate the HTML for each page in the site
    // 1. Generate the index page
    // 2. Generate the other pages
    // 3. Generate the blog posts
    let mut failures = 0;
    let (mut written, mut skipped) = (0, 0);
    for page in site.all_pages() {
        match site.generate_page(page, &tera, &mut cache, &build_fingerprint) {
            Ok(true) => {
                info!("Generated {}.html", page.summary.name);
                written += 1;
            }
            Ok(false) => {
                debug!("Skipping unchanged page {}.html", page.summary.name);
                skipped += 1;
            }
            Err(e) => {
                error!(
                    "Failed to generate page {:?}: {}",
                    page.source,
                    error_chain(e.as_ref())
                );
                failures += 1;
            }
        }
    }
    info!("Pages: {written} generated, {skipped} unchanged");

    let mut redirects = 0;
    for page in site.all_pages() {
        match site.generate_aliases(page, &mut cache, &build_fingerprint) {
            Ok(count) => redirects += count,
            Err(e) => {
                error!(
                    "Failed to generate redirects for {:?}: {}",
                    page.source,
                    error_chain(e.as_ref())
                );
                failures += 1;
            }
        }
    }
    if redirects > 0 {
        info!("Redirects: {redirects} generated");
    }

    let tailwind_result = if args.no_css {
        info!("Skipping Tailwind build (--no-css)");
        Ok(())
    } else {
        build_tailwind(&site)
    };

    // Save the cache (and remove stale outputs) even when some pages failed so that successful
    // work is not repeated. Failed outputs were invalidated and will be retried.
    cache.finish()?;
    tailwind_result?;

    if failures > 0 {
        return Err(format!("{failures} page(s) failed to generate").into());
    }

    info!("Static site generated in {:?}", output_dir);
    Ok(())
}

fn main() -> ExitCode {
    // Initialize the logger (which uses an environment variable to correspondingly toggle)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("Build failed: {}", error_chain(e.as_ref()));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    const PAGE_TEMPLATE: &str = "{{ title }}|{{ description }}|{% for p in pages %}{{ p.url | safe }}={{ p.title }},{% endfor %}|{{ content | safe }}";

    fn site(dir: &TempDir) -> Site {
        let config = Config {
            metadata: SiteMetadata {
                base_url: "https://example.com".into(),
                author: "Author".into(),
                description: "Site description".into(),
            },
            paths: Paths {
                content_dir: dir.path().join("content"),
                template_dir: dir.path().join("templates"),
                output_dir: dir.path().join("build"),
                static_dir: dir.path().join("static"),
            },
            build: Build {
                minify_html: false,
                ..Build::default()
            },
        };
        fs::create_dir_all(dir.path().join("content")).unwrap();
        fs::create_dir_all(dir.path().join("build")).unwrap();
        Site::new(config, dir.path().join("config.yml"))
    }

    fn tera(dir: &TempDir) -> Tera {
        for name in ["index", "page", "post"] {
            dir.write(&format!("templates/{name}.html"), PAGE_TEMPLATE);
        }
        dir.write(
            "templates/partials/image.html",
            "<img src=\"{{ src | safe }}\">",
        );
        Tera::new(dir.path().join("templates/**/*.html").to_str().unwrap()).unwrap()
    }

    fn md(title: &str, body: &str) -> String {
        format!("---\ntitle: \"{title}\"\n---\n{body}\n")
    }

    fn names(pages: &[Page]) -> Vec<&str> {
        pages.iter().map(|p| p.summary.name.as_str()).collect()
    }

    // Generate every page of the site, returning the names of the pages that were written
    fn generate_all(site: &Site, tera: &Tera, cache: &mut BuildCache) -> Vec<String> {
        let fingerprint = Fingerprint::new();
        site.all_pages()
            .filter(|page| site.generate_page(page, tera, cache, &fingerprint).unwrap())
            .map(|page| page.summary.name.clone())
            .collect()
    }

    fn args(raw: &[&str]) -> Result<Option<Args>, String> {
        parse_args(raw.iter().map(|s| s.to_string()))
    }

    #[test]
    fn error_chain_includes_causes() {
        let dir = TempDir::new();
        dir.write("templates/page.html", "{{ undefined_variable }}");
        let tera = Tera::new(dir.path().join("templates/*.html").to_str().unwrap()).unwrap();
        let error = tera.render("page.html", &Context::new()).unwrap_err();
        let message = error_chain(&error);
        assert!(message.contains("page.html"), "{message}");
        assert!(message.contains("undefined_variable"), "{message}");
    }

    #[test]
    fn page_types_come_from_folder_names() {
        assert_eq!(PageType::from_folder_name("index"), PageType::Index);
        assert_eq!(PageType::from_folder_name("page"), PageType::Page);
        assert_eq!(PageType::from_folder_name("post"), PageType::Post);
        assert_eq!(PageType::from_folder_name("book"), PageType::Unknown);
        assert_eq!(PageType::Post.template_name(), Some("post.html"));
        assert_eq!(PageType::Unknown.template_name(), None);
    }

    #[test]
    fn parses_command_line_flags() {
        let parsed = args(&[]).unwrap().unwrap();
        assert!(!parsed.force && !parsed.clean && !parsed.no_css);

        let parsed = args(&["--force", "--clean", "--no-css"]).unwrap().unwrap();
        assert!(parsed.force && parsed.clean && parsed.no_css);
        assert!(args(&["-f"]).unwrap().unwrap().force);

        assert!(args(&["--help"]).unwrap().is_none());
        assert!(args(&["--bogus"]).unwrap_err().contains("--bogus"));
    }

    #[test]
    fn build_section_of_config_is_optional() {
        let dir = TempDir::new();
        let path = dir.write(
            "config.yml",
            "metadata: {base_url: x, author: a, description: d}\n\
             paths: {content_dir: c, template_dir: t, output_dir: o, static_dir: s}\n",
        );
        let config = load_yaml_config(&path).unwrap();
        assert!(config.build.minify_html && config.build.cache && !config.build.generate_sitemap);

        dir.write(
            "config.yml",
            "metadata: {base_url: x, author: a, description: d}\n\
             paths: {content_dir: c, template_dir: t, output_dir: o, static_dir: s}\n\
             build: {cache: false}\n",
        );
        let config = load_yaml_config(&path).unwrap();
        assert!(config.build.minify_html && !config.build.cache);
    }

    #[test]
    fn invalid_or_missing_config_is_a_clear_error() {
        let dir = TempDir::new();
        let missing = load_yaml_config(&dir.path().join("nope.yml")).unwrap_err();
        assert!(missing.to_string().contains("Could not read config file"));

        let path = dir.write("config.yml", "metadata: {}\n");
        let invalid = load_yaml_config(&path).unwrap_err();
        assert!(invalid.to_string().contains("Invalid config file"));
    }

    #[test]
    fn paths_are_relative_to_the_config_file() {
        let paths = reconcile_configuration_directory_paths(
            Path::new("/site"),
            Paths {
                content_dir: "./content/".into(),
                template_dir: "templates".into(),
                output_dir: "/absolute/build".into(),
                static_dir: "static".into(),
            },
        );
        assert_eq!(paths.content_dir, Path::new("/site/./content/"));
        assert_eq!(paths.template_dir, Path::new("/site/templates"));
        assert_eq!(paths.output_dir, Path::new("/absolute/build"));
    }

    fn page_with(frontmatter: &str) -> String {
        format!("---\ntitle: T\n{frontmatter}\n---\n")
    }

    #[test]
    fn loads_pages_by_folder() {
        let dir = TempDir::new();
        dir.write("content/index/index.md", md("Home", "hi"));
        dir.write("content/page/about.md", md("About", ""));
        dir.write("content/post/20200101-old.md", md("Old", ""));
        dir.write("content/post/20240101-new.md", md("New", ""));
        let mut site = site(&dir);
        load_content(&mut site).unwrap();

        assert_eq!(site.index.as_ref().unwrap().summary.name, "index");
        assert_eq!(names(&site.pages), ["about"]);
        // Posts: descending file name order, so newest first for date-prefixed names
        assert_eq!(names(&site.posts), ["20240101-new", "20200101-old"]);
    }

    #[test]
    fn pages_are_sorted_by_order_then_name() {
        let dir = TempDir::new();
        dir.write("content/page/contact.md", page_with("order: 3"));
        dir.write("content/page/university.md", page_with("order: 1"));
        dir.write("content/page/experience.md", page_with("order: 2"));
        dir.write("content/page/zebra.md", md("Z", ""));
        dir.write("content/page/404.md", md("Not Found", ""));
        dir.write("content/page/negative.md", page_with("order: -1"));
        let mut site = site(&dir);
        load_content(&mut site).unwrap();

        assert_eq!(
            names(&site.pages),
            [
                "negative",
                "university",
                "experience",
                "contact",
                "404",
                "zebra"
            ]
        );
    }

    #[test]
    fn slug_overrides_the_output_name() {
        let dir = TempDir::new();
        dir.write("content/page/1contact.md", page_with("slug: contact"));
        let mut site = site(&dir);
        load_content(&mut site).unwrap();

        let summary = &site.pages[0].summary;
        assert_eq!(summary.name, "contact");
        assert_eq!(summary.url.as_deref(), Some("/contact.html"));
    }

    #[test]
    fn invalid_names_are_rejected() {
        for (frontmatter, field) in [
            ("slug: ../escape", "page name"),
            ("slug: \"a/b\"", "page name"),
            ("slug: \".hidden\"", "page name"),
            ("slug: \"has space\"", "page name"),
            ("aliases: [\"../x\"]", "alias"),
            ("aliases: [\"\"]", "alias"),
        ] {
            let dir = TempDir::new();
            dir.write("content/page/a.md", page_with(frontmatter));
            let error = load_content(&mut site(&dir)).unwrap_err().to_string();
            assert!(
                error.contains(&format!("Invalid {field}")),
                "{frontmatter}: {error}"
            );
        }
    }

    #[test]
    fn aliases_accept_names_with_or_without_extension() {
        let dir = TempDir::new();
        dir.write(
            "content/page/contact.md",
            page_with("aliases: [1contact, old-contact.html]"),
        );
        let mut site = site(&dir);
        load_content(&mut site).unwrap();
        assert_eq!(site.pages[0].aliases, ["1contact", "old-contact"]);
    }

    #[test]
    fn aliases_must_not_clash_with_pages_or_each_other() {
        let dir = TempDir::new();
        dir.write("content/page/a.md", page_with("aliases: [b]"));
        dir.write("content/page/b.md", md("B", ""));
        let error = load_content(&mut site(&dir)).unwrap_err().to_string();
        assert!(error.contains("\"b\""), "{error}");

        let dir = TempDir::new();
        dir.write("content/page/a.md", page_with("aliases: [old]"));
        dir.write("content/page/b.md", page_with("aliases: [old]"));
        let error = load_content(&mut site(&dir)).unwrap_err().to_string();
        assert!(error.contains("\"old\""), "{error}");
    }

    #[test]
    fn aliases_generate_redirect_pages() {
        let dir = TempDir::new();
        dir.write("content/page/contact.md", page_with("aliases: [1contact]"));
        let mut site = site(&dir);
        load_content(&mut site).unwrap();
        let cache_path = dir.path().join("cache.json");
        let fingerprint = Fingerprint::new();

        let mut cache = BuildCache::load(cache_path.clone(), dir.path().join("build"), true);
        assert_eq!(
            site.generate_aliases(&site.pages[0], &mut cache, &fingerprint)
                .unwrap(),
            1
        );
        cache.finish().unwrap();

        let html = fs::read_to_string(dir.path().join("build/1contact.html")).unwrap();
        assert!(
            html.contains("http-equiv=\"refresh\" content=\"0; url=/contact.html\""),
            "{html}"
        );
        assert!(
            html.contains("rel=\"canonical\" href=\"https://example.com/contact.html\""),
            "{html}"
        );
        assert!(html.contains("noindex"), "{html}");

        // Cached on the next build
        let mut cache = BuildCache::load(cache_path, dir.path().join("build"), true);
        assert_eq!(
            site.generate_aliases(&site.pages[0], &mut cache, &fingerprint)
                .unwrap(),
            0
        );
    }

    #[test]
    fn page_metadata_comes_from_frontmatter() {
        let dir = TempDir::new();
        dir.write(
            "content/post/a.md",
            "---\ntitle: T\ndate: \"2024-01-01\"\ndescription: Desc\ntags: [x, y]\n---\nBody\n",
        );
        let mut site = site(&dir);
        load_content(&mut site).unwrap();

        let page = &site.posts[0];
        assert_eq!(page.summary.title.as_deref(), Some("T"));
        assert_eq!(page.summary.date.as_deref(), Some("2024-01-01"));
        assert_eq!(page.summary.description.as_deref(), Some("Desc"));
        assert_eq!(
            page.summary.tags,
            Some(vec!["x".to_string(), "y".to_string()])
        );
        assert_eq!(page.summary.url.as_deref(), Some("/a.html"));
        assert_eq!(page.content.trim(), "Body");
    }

    #[test]
    fn skips_files_without_frontmatter_unknown_folders_and_non_markdown() {
        let dir = TempDir::new();
        dir.write("content/page/no-frontmatter.md", "just text");
        dir.write("content/book/book.md", md("Book", ""));
        dir.write("content/page/notes.txt", "not markdown");
        dir.write("content/page/real.md", md("Real", ""));
        let mut site = site(&dir);
        load_content(&mut site).unwrap();

        assert_eq!(names(&site.pages), ["real"]);
        assert!(site.posts.is_empty() && site.index.is_none());
    }

    #[test]
    fn invalid_frontmatter_is_an_error() {
        let dir = TempDir::new();
        dir.write("content/page/bad.md", "---\ntitle: [unclosed\n---\n");
        let error = load_content(&mut site(&dir)).unwrap_err().to_string();
        assert!(error.contains("bad.md"), "{error}");
    }

    #[test]
    fn duplicate_page_names_are_an_error() {
        let dir = TempDir::new();
        dir.write("content/page/same.md", md("A", ""));
        dir.write("content/post/same.md", md("B", ""));
        let error = load_content(&mut site(&dir)).unwrap_err().to_string();
        assert!(error.contains("same"), "{error}");
    }

    #[test]
    fn generates_pages_with_listing_and_description() {
        let dir = TempDir::new();
        dir.write(
            "content/page/a.md",
            "---\ntitle: A\norder: 2\n---\n# Hello\n",
        );
        dir.write(
            "content/page/b.md",
            "---\ntitle: B\norder: 1\ndescription: Own description\n---\n",
        );
        let mut site = site(&dir);
        load_content(&mut site).unwrap();
        let tera = tera(&dir);
        let mut cache = BuildCache::load(
            dir.path().join("cache.json"),
            dir.path().join("build"),
            true,
        );
        generate_all(&site, &tera, &mut cache);

        let a = fs::read_to_string(dir.path().join("build/a.html")).unwrap();
        assert!(
            a.starts_with("A|Site description|/b.html=B,/a.html=A,|"),
            "{a}"
        );
        assert!(a.contains("<h1 class="), "{a}");
        let b = fs::read_to_string(dir.path().join("build/b.html")).unwrap();
        assert!(b.starts_with("B|Own description|"), "{b}");
    }

    #[test]
    fn current_year_is_available_and_invalidates_the_cache() {
        let dir = TempDir::new();
        dir.write("content/page/a.md", md("A", "a"));
        tera(&dir);
        dir.write("templates/page.html", "Copyright {{ current_year }}");
        let tera = Tera::new(dir.path().join("templates/**/*.html").to_str().unwrap()).unwrap();
        let cache_path = dir.path().join("cache.json");
        let build = |year: i32| {
            let mut site = site(&dir);
            site.current_year = year;
            load_content(&mut site).unwrap();
            let mut cache = BuildCache::load(cache_path.clone(), dir.path().join("build"), true);
            let written = generate_all(&site, &tera, &mut cache);
            cache.finish().unwrap();
            written
        };

        assert_eq!(build(2026), ["a"]);
        assert!(build(2026).is_empty());
        assert_eq!(build(2027), ["a"], "a new year rebuilds the page");
        let html = fs::read_to_string(dir.path().join("build/a.html")).unwrap();
        assert_eq!(html, "Copyright 2027");
    }

    #[test]
    fn minifies_html_when_enabled() {
        let dir = TempDir::new();
        dir.write("content/page/a.md", md("A", "text"));
        let mut site = site(&dir);
        site.configuration.build.minify_html = true;
        load_content(&mut site).unwrap();
        tera(&dir);
        dir.write(
            "templates/page.html",
            "<html>\n  <body>\n    {{ content | safe }}\n  </body>\n</html>",
        );
        let tera = Tera::new(dir.path().join("templates/**/*.html").to_str().unwrap()).unwrap();
        let mut cache = BuildCache::load(
            dir.path().join("cache.json"),
            dir.path().join("build"),
            false,
        );
        generate_all(&site, &tera, &mut cache);

        let html = fs::read_to_string(dir.path().join("build/a.html")).unwrap();
        assert!(!html.contains("\n  "), "{html}");
    }

    #[test]
    fn cache_skips_unchanged_pages_and_rebuilds_dependents() {
        let dir = TempDir::new();
        dir.write("content/page/a.md", md("A", "a"));
        dir.write("content/page/b.md", md("B", "b"));
        let tera = tera(&dir);
        let cache_path = dir.path().join("cache.json");
        let build = |enabled: bool| {
            let mut site = site(&dir);
            load_content(&mut site).unwrap();
            let mut cache = BuildCache::load(cache_path.clone(), dir.path().join("build"), enabled);
            let mut written = generate_all(&site, &tera, &mut cache);
            cache.finish().unwrap();
            written.sort();
            written
        };

        assert_eq!(build(true), ["a", "b"]);
        assert!(build(true).is_empty(), "nothing changed");

        // Body change: only that page
        dir.write("content/page/a.md", md("A", "changed"));
        assert_eq!(build(true), ["a"]);

        // Title change: every page lists it in the navigation
        dir.write("content/page/a.md", md("A2", "changed"));
        assert_eq!(build(true), ["a", "b"]);

        // Deleted output is regenerated
        fs::remove_file(dir.path().join("build/b.html")).unwrap();
        assert_eq!(build(true), ["b"]);

        // Cache disabled: everything
        assert_eq!(build(false), ["a", "b"]);

        // Deleted source: output removed
        fs::remove_file(dir.path().join("content/page/b.md")).unwrap();
        build(true);
        assert!(!dir.path().join("build/b.html").exists());
        assert!(dir.path().join("build/a.html").exists());
    }

    #[test]
    fn build_fingerprint_changes_invalidate_every_page() {
        let dir = TempDir::new();
        dir.write("content/page/a.md", md("A", "a"));
        let mut site = site(&dir);
        load_content(&mut site).unwrap();
        let tera = tera(&dir);
        let mut cache = BuildCache::load(
            dir.path().join("cache.json"),
            dir.path().join("build"),
            true,
        );
        let page = &site.pages[0];

        let mut one = Fingerprint::new();
        one.add("templates v1");
        let mut two = Fingerprint::new();
        two.add("templates v2");
        assert!(site.generate_page(page, &tera, &mut cache, &one).unwrap());
        cache.finish().unwrap();

        let mut cache = BuildCache::load(
            dir.path().join("cache.json"),
            dir.path().join("build"),
            true,
        );
        assert!(!site.generate_page(page, &tera, &mut cache, &one).unwrap());
        assert!(site.generate_page(page, &tera, &mut cache, &two).unwrap());
    }

    #[test]
    fn template_errors_are_reported() {
        let dir = TempDir::new();
        dir.write("content/page/a.md", md("A", "a"));
        let mut site = site(&dir);
        load_content(&mut site).unwrap();
        let tera = tera(&dir);
        dir.write("templates/page.html", "{{ missing_variable }}");
        let broken = Tera::new(dir.path().join("templates/**/*.html").to_str().unwrap()).unwrap();
        let mut cache = BuildCache::load(
            dir.path().join("cache.json"),
            dir.path().join("build"),
            true,
        );

        let fingerprint = Fingerprint::new();
        assert!(site
            .generate_page(&site.pages[0], &broken, &mut cache, &fingerprint)
            .is_err());
        assert!(site
            .generate_page(&site.pages[0], &tera, &mut cache, &fingerprint)
            .unwrap());
    }

    #[test]
    fn directory_fingerprint_tracks_contents_and_names_but_not_hidden_files() {
        let dir = TempDir::new();
        dir.write("t/a.html", "a");
        dir.write("t/partials/b.html", "b");
        let t = dir.path().join("t");
        let original = directory_fingerprint(&t).unwrap();
        assert_eq!(original, directory_fingerprint(&t).unwrap());

        dir.write("t/.DS_Store", "junk");
        assert_eq!(original, directory_fingerprint(&t).unwrap());

        dir.write("t/partials/b.html", "changed");
        let changed = directory_fingerprint(&t).unwrap();
        assert_ne!(original, changed);

        fs::rename(t.join("partials/b.html"), t.join("partials/c.html")).unwrap();
        assert_ne!(changed, directory_fingerprint(&t).unwrap());
    }

    #[test]
    fn clean_removes_the_output_directory() {
        let dir = TempDir::new();
        let site = site(&dir);
        fs::create_dir_all(dir.path().join("templates")).unwrap();
        fs::create_dir_all(dir.path().join("static")).unwrap();
        dir.write("config.yml", "");
        dir.write("build/old.html", "old");

        clean_output_dir(&site.configuration, &site.config_path).unwrap();
        assert!(!dir.path().join("build").exists());
        // Cleaning a missing output directory is fine
        clean_output_dir(&site.configuration, &site.config_path).unwrap();
    }

    #[test]
    fn clean_refuses_to_delete_site_inputs() {
        let dir = TempDir::new();
        let mut site = site(&dir);
        fs::create_dir_all(dir.path().join("templates")).unwrap();
        fs::create_dir_all(dir.path().join("static")).unwrap();
        dir.write("config.yml", "");
        // Output directory set to the project root, which contains everything
        site.configuration.paths.output_dir = dir.path().to_path_buf();

        let error = clean_output_dir(&site.configuration, &site.config_path).unwrap_err();
        assert!(error.to_string().contains("Refusing"), "{error}");
        assert!(dir.path().join("config.yml").exists());
    }
}
