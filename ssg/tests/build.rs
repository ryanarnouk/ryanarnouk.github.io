// End-to-end tests: run the compiled `ssg` binary against a small fixture site in a temporary
// directory (never the real site content). Tailwind is skipped with --no-css so these tests do
// not need Node.js.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ssg-e2e-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&root);
        let fixture = Fixture(root);

        fixture.write(
            "config.yml",
            "metadata:\n  base_url: \"https://example.com\"\n  author: \"Test Author\"\n  description: \"Test site\"\n\
             paths:\n  content_dir: \"./content/\"\n  template_dir: \"./templates/\"\n  output_dir: \"./build/\"\n  static_dir: \"./static/\"\n\
             build:\n  minify_html: false\n  cache: true\n",
        );

        let layout = |body: &str| {
            format!(
                "<!doctype html><html><head><title>{{{{ title }}}}</title>\
                 <meta name=\"description\" content=\"{{{{ description }}}}\"></head><body>\
                 {{% include \"partials/header.html\" %}}{body}<footer>{{{{ current_year }}}}</footer></body></html>"
            )
        };
        fixture.write(
            "templates/partials/header.html",
            "<nav>{% for page in pages %}<a href=\"{{ page.url }}\">{{ page.title }}</a>{% endfor %}</nav>",
        );
        fixture.write(
            "templates/partials/image.html",
            "<img src=\"{{ src | safe }}\" alt=\"{{ alt }}\">",
        );
        fixture.write(
            "templates/index.html",
            &layout("{{ content | safe }}<ul>{% for post in posts %}<li>{{ post.title }}</li>{% endfor %}</ul>"),
        );
        fixture.write("templates/page.html", &layout("{{ content | safe }}"));
        fixture.write(
            "templates/post.html",
            &layout("<h1>{{ title }}</h1>{{ content | safe }}"),
        );

        fixture.write(
            "content/index/index.md",
            "---\ntitle: Home\n---\nWelcome ![logo](images/logo.png)\n",
        );
        fixture.write(
            "content/page/about.md",
            "---\ntitle: About\norder: 2\naliases: [1about]\n---\n## About me\n",
        );
        fixture.write(
            "content/page/contact.md",
            "---\ntitle: Contact\norder: 1\n---\n<me@example.com>\n",
        );
        fixture.write(
            "content/post/20240101-first.md",
            "---\ntitle: First post\ndescription: The first one\n---\n```html\n<div>escaped</div>\n```\n",
        );
        fixture.write("content/book/draft.md", "no frontmatter yet");

        let logo = fixture.path("static/images/logo.png");
        fs::create_dir_all(logo.parent().unwrap()).unwrap();
        image::RgbImage::new(8, 8).save(&logo).unwrap();
        fixture.write("static/icons/favicon.ico", "ico bytes");
        fixture.write("static/.DS_Store", "junk");

        fixture
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.path(relative))
            .unwrap_or_else(|e| panic!("could not read {relative}: {e}"))
    }

    fn exists(&self, relative: &str) -> bool {
        self.path(relative).exists()
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ssg"))
            .args(["--no-css"])
            .args(args)
            .env("CONFIG_PATH", self.path("config.yml"))
            .env("RUST_LOG", "info")
            .output()
            .unwrap()
    }

    // Run a build that must succeed, returning its log output
    fn build(&self, args: &[&str]) -> String {
        let output = self.run(args);
        let log = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(output.status.success(), "build failed:\n{log}");
        log
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pages_line(log: &str) -> &str {
    log.lines()
        .find(|l| l.contains("Pages:"))
        .map(|l| &l[l.find("Pages:").unwrap()..])
        .unwrap_or_else(|| panic!("no Pages line in:\n{log}"))
}

#[test]
fn builds_the_whole_site() {
    let site = Fixture::new("whole");
    let log = site.build(&[]);
    assert_eq!(pages_line(&log), "Pages: 4 generated, 0 unchanged");

    for page in ["index", "about", "contact", "20240101-first"] {
        assert!(
            site.exists(&format!("build/{page}.html")),
            "{page}.html missing"
        );
    }
    // Navigation lists every page by `order`, with root-relative links
    let index = site.read("build/index.html");
    let nav = &index[index.find("<nav>").unwrap()..index.find("</nav>").unwrap()];
    assert_eq!(
        nav,
        "<nav><a href=\"&#x2F;contact.html\">Contact</a><a href=\"&#x2F;about.html\">About</a>"
    );
    assert!(index.contains("<li>First post</li>"));
    assert!(index.contains("src=\"./static/images/logo.webp\""));

    // Aliases redirect old URLs to the page's current URL
    let redirect = site.read("build/1about.html");
    assert!(
        redirect.contains("content=\"0; url=/about.html\""),
        "{redirect}"
    );
    assert!(
        redirect.contains("href=\"https://example.com/about.html\""),
        "{redirect}"
    );
    assert!(log.contains("Redirects: 1 generated"), "{log}");

    // The footer year is filled in at build time
    let year = chrono::Datelike::year(&chrono::Local::now());
    assert!(
        index.contains(&format!("<footer>{year}</footer>")),
        "{index}"
    );

    // Page description overrides the site description
    assert!(site
        .read("build/20240101-first.html")
        .contains("content=\"The first one\""));
    assert!(site
        .read("build/about.html")
        .contains("content=\"Test site\""));
    // Code is escaped, email links work
    assert!(site
        .read("build/20240101-first.html")
        .contains("&lt;div&gt;escaped&lt;/div&gt;"));
    assert!(site
        .read("build/contact.html")
        .contains("href=\"mailto:me@example.com\""));

    // Static files: images converted, other files copied, hidden files skipped
    assert!(site.exists("build/static/images/logo.webp"));
    assert!(!site.exists("build/static/images/logo.png"));
    assert_eq!(site.read("build/static/icons/favicon.ico"), "ico bytes");
    assert!(!site.exists("build/static/.DS_Store"));

    // Files without frontmatter are reported, and the cache is kept out of the deployed folder
    assert!(log.contains("draft.md\": it has no frontmatter"), "{log}");
    assert!(site.exists(".ssg-cache.json"));
    assert!(!site.exists("build/.ssg-cache.json"));
}

#[test]
fn incremental_rebuilds_only_touch_what_changed() {
    let site = Fixture::new("incremental");
    site.build(&[]);

    let log = site.build(&[]);
    assert_eq!(pages_line(&log), "Pages: 0 generated, 4 unchanged");
    assert!(!log.contains("Optimizing static image"), "{log}");

    // Body-only change rebuilds one page
    site.write(
        "content/page/about.md",
        "---\ntitle: About\norder: 2\naliases: [1about]\n---\nNew text\n",
    );
    assert_eq!(
        pages_line(&site.build(&[])),
        "Pages: 1 generated, 3 unchanged"
    );

    // Title change rebuilds every page (they all list it in the navigation)
    site.write(
        "content/page/about.md",
        "---\ntitle: About me\norder: 2\naliases: [1about]\n---\nNew text\n",
    );
    assert_eq!(
        pages_line(&site.build(&[])),
        "Pages: 4 generated, 0 unchanged"
    );
    assert!(site.read("build/index.html").contains(">About me</a>"));

    // Template change rebuilds every page
    site.write("templates/partials/header.html", "<nav>changed</nav>");
    assert_eq!(
        pages_line(&site.build(&[])),
        "Pages: 4 generated, 0 unchanged"
    );

    // A deleted output is regenerated
    fs::remove_file(site.path("build/about.html")).unwrap();
    assert_eq!(
        pages_line(&site.build(&[])),
        "Pages: 1 generated, 3 unchanged"
    );

    // --force rebuilds everything
    assert_eq!(
        pages_line(&site.build(&["--force"])),
        "Pages: 4 generated, 0 unchanged"
    );

    // A changed image is re-optimized
    image::RgbImage::new(16, 16)
        .save(site.path("static/images/logo.png"))
        .unwrap();
    assert!(site.build(&[]).contains("Optimizing static image"));
}

#[test]
fn deleted_sources_are_removed_from_the_build() {
    let site = Fixture::new("deleted");
    site.build(&[]);
    fs::remove_file(site.path("content/page/about.md")).unwrap();
    fs::remove_file(site.path("static/icons/favicon.ico")).unwrap();
    site.build(&[]);

    assert!(!site.exists("build/about.html"));
    assert!(
        !site.exists("build/1about.html"),
        "redirect of a deleted page is removed"
    );
    assert!(!site.exists("build/static/icons/favicon.ico"));
    assert!(!site.read("build/index.html").contains("About"));
}

#[test]
fn clean_rebuilds_from_scratch() {
    let site = Fixture::new("clean");
    site.build(&[]);
    site.write("build/leftover.html", "not generated by the ssg");

    let log = site.build(&["--clean"]);
    assert_eq!(pages_line(&log), "Pages: 4 generated, 0 unchanged");
    assert!(!site.exists("build/leftover.html"));
    assert!(site.exists("build/index.html"));
}

#[test]
fn corrupt_cache_falls_back_to_a_full_build() {
    let site = Fixture::new("corrupt");
    site.build(&[]);
    site.write(".ssg-cache.json", "{ definitely not json");
    let log = site.build(&[]);
    assert!(log.contains("corrupt"), "{log}");
    assert_eq!(pages_line(&log), "Pages: 4 generated, 0 unchanged");
}

#[test]
fn failures_exit_with_an_error() {
    let site = Fixture::new("failures");

    // Invalid frontmatter
    site.write("content/page/bad.md", "---\ntitle: [unclosed\n---\n");
    let output = site.run(&[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("bad.md"));
    fs::remove_file(site.path("content/page/bad.md")).unwrap();

    // Broken template
    site.write("templates/page.html", "{{ undefined_variable }}");
    let output = site.run(&[]);
    assert!(!output.status.success());
    let log = String::from_utf8_lossy(&output.stderr);
    // Both pages using page.html fail, and the error names the actual problem
    assert!(log.contains("2 page(s) failed to generate"), "{log}");
    assert!(log.contains("undefined_variable"), "{log}");

    // Missing config
    let output = Command::new(env!("CARGO_BIN_EXE_ssg"))
        .env("CONFIG_PATH", site.path("missing.yml"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Could not read config file"));
}

#[test]
fn failed_pages_are_retried_on_the_next_build() {
    let site = Fixture::new("retry");
    site.build(&[]);
    let template = site.read("templates/page.html");

    site.write("templates/page.html", "{{ undefined_variable }}");
    assert!(!site.run(&[]).status.success());

    site.write("templates/page.html", &template);
    let log = site.build(&[]);
    assert!(!pages_line(&log).starts_with("Pages: 0 generated"), "{log}");
    assert!(site
        .read("build/about.html")
        .contains("<title>About</title>"));
}

#[test]
fn command_line_usage() {
    let help = Command::new(env!("CARGO_BIN_EXE_ssg"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--force"));

    let bad = Command::new(env!("CARGO_BIN_EXE_ssg"))
        .arg("--bogus")
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("Unknown argument: --bogus"));
}

#[test]
fn removing_an_alias_removes_its_redirect() {
    let site = Fixture::new("alias-removed");
    site.build(&[]);
    assert!(site.exists("build/1about.html"));

    site.write(
        "content/page/about.md",
        "---\ntitle: About\norder: 2\n---\n## About me\n",
    );
    site.build(&[]);
    assert!(!site.exists("build/1about.html"));
    assert!(site.exists("build/about.html"));
}
