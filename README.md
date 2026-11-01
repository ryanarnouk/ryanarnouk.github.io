# Personal Website

My personal website, bundled with a static site generator written in Rust.

## Motivation

I want to develop a robust and solid site without worrying about maintenance and package upgrades for the foreseeable future.

My old personal pages usually rely on too many tools and the pace of development and upgrade cycles outpaces the amount I am able to take a look. I end up having old projects with terribly old code bases that need upgrading.

At times, it was even easier to settle with remaking my personal page altogether. My hope with offering a greater level of control over the SSG in this version of my site is that this will not need to happen as often.

## SSG

I originally intended to write the static site generator component of the project in OCaml but later settled for Rust for a greater array of libraries/frameworks and previous familiarity.

Warning: the SSG is not fully tested (outside the scope of my site specifically) and as such I kept it bundled within the same repository as my site. It is not built for a scope beyond my specific use case.

### Libraries
- Pulldown-cmark (markdown parsing)
- Tera (template engine)
- Walkdir (recursively walk through directories)
- Graymatter (frontmatter parsing)
- Serde (and Serde JSON for the build cache and Serde YAML for the site configuration)
- Minify-html (provide a minified output HTML for extra optimizations set in the config YAML file)
- Image (convert static images to WebP for a better file size)
- Log & Env Logger (set level of verbose logging desired before run)
- Blake3 (fingerprint the inputs of every generated file and compare with the cache to only regenerate a file when something it depends on has changed)

A primary goal with this project is to remove as many dependencies as possible and focus on a more simple (yet complete) solution. This included parting from frameworks like React that I typically use on my sites and minimizing any JavaScript needed to power the site (it's a static site after all).

## Site

## Content

Markdown files live in `content/<type>/`, where the folder picks the template: `index`, `page` or `post`. Each file needs YAML frontmatter. Supported fields:

| Field | Purpose |
|---|---|
| `title`, `description`, `date`, `tags` | Page metadata (`description` also fills the meta description tag) |
| `order` | Position in the navigation, lowest first. Pages without one are listed after, alphabetically |
| `slug` | Output name (`<slug>.html`). Defaults to the file name |
| `aliases` | Old names for the page, e.g. `[1contact]`. Each gets a small page that redirects to the current URL |

Posts are listed newest first by file name, so name them with a date prefix (`20250506-title.md`).

## Build cache

When `build.cache` is enabled in `config.yml`, the SSG stores a fingerprint of everything that went into each generated file in `.ssg-cache.json` (next to `config.yml`, so it is never deployed). A page is only regenerated when its content, the templates, the site configuration, the list of other pages (navigation, recent posts) or the SSG itself changes, and only if the output file is missing or out of date. Static files are re-optimized only when they change. Outputs whose source was deleted are removed from the build folder.

- `cargo run -- --force` rebuilds everything, ignoring the cache
- `cargo run -- --clean` deletes the build folder and cache before building (used by the GitHub Action)
- `cargo run -- --no-css` skips the Tailwind build (no Node.js needed)

## Testing

From the `ssg` folder, run `cargo test`. This runs unit tests for each module and end-to-end tests (`ssg/tests/build.rs`) that build a small fixture site in a temporary folder, never the real content. The GitHub Action runs the tests before every deploy.

To preview the real site, build it and serve the `build` folder: `cargo run && python3 -m http.server 8000 -d ../build`, then open http://localhost:8000.

## Setup

### Cargo Run (testing)

To generate the site, change your working-directory to the `ssg` folder. From there, you can run `cargo run`.

Info logs are shown by default. For debug logs (useful for noting what files are skipped over based on the cache), run `RUST_LOG=DEBUG cargo run`

### Cargo Build (release versions)

For release versions, run `cargo build --release`. Afterwards, before running the binary, set an environment variable to represent the location of the configuration YAML file (`CONFIG_PATH`).

The `CONFIG_PATH` environment variable will be used instead of a relative path from the working directory, which happens when using `cargo run`.
