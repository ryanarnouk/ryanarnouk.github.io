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
- Blake3 (hash file content and compare with cache to only generate a file when it has been changed)

A primary goal with this project is to remove as many dependencies as possible and focus on a more simple (yet complete) solution. This included parting from frameworks like React that I typically use on my sites and minimizing any JavaScript needed to power the site (it's a static site after all).

## Site

## TODO

Add force flag before running to rebuild every file (ignoring cache) and/or a way to clear the cache from the CLI.

## Setup

### Cargo Run (testing)

To generate the site, change your working-directory to the `ssg` folder. From there, you can run `cargo run`.

For debug/info logs to appear (useful for noting what files are being rebuilt or skipped over based on the cache), run `RUST_LOG=DEBUG cargo run`

### Cargo Build (release versions)

For release versions, run `cargo build --release`. Afterwards, before running the binary, set an environment variable to represent the location of the configuration YAML file (`CONFIG_PATH`).

The `CONFIG_PATH` environment variable will be used instead of a relative path from the working directory, which happens when using `cargo run`.
