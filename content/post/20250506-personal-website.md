---
title: "Remaking My Personal Website"
date: "05-06-2025"
tags: ["Rust", "Web Development"]
---

An overview into my own personal website, bundled with a static site generator written in Rust.

## Motivation

I want to develop a robust and solid site without worrying about maintenance and package upgrades for the foreseeable future. I often neglect to keep my tech stacks up to date on my previous sites. 

My old personal pages usually rely on too many tools and the pace of development and upgrade cycles outpaces the amount of time I am able to spend maintaining the project. I end up having old projects with terribly old code bases that need upgrading.

At times, it was even easier to settle with remaking my personal page altogether. My hope with offering a greater level of control over the SSG in this version of my site is that this will not need to happen as often.

## Static Site Generator

I originally intended to write the static site generator component of the project in OCaml but later settled for Rust for familiarity and libraries that are available in the Rust ecosystem.

However, the expansive ecosystem of Rust libraries presents a challenge of where to strike the balance between libraries (and other dependencies) and my goal of having a simplistic website that had very little reliance on code that will be constantly be updated and may have vulnerabilities. On the other hand, writing everything myself would have been way too long (writing the parser for markdown alone would have taken a while).

Because of this, I settled for a balance. While I could use libraries, most of the system would be well understood by me to make updates later on the future (and make dependency upgrades as needed). And, nothing is using "flashy" web development technologies that come and go (except for TailwindCSS arguably).

### Libraries

To avoid my issues using web development frameworks and libraries in the past, and falling behind in upgrades, I tried to keep libraries needed for this project to a minimum. Most of which probably wouldn't require any ground-breaking changes in the near future (I hope).

In total I used the following libraries (installed via Cargo):
- Pulldown-cmark (parser for Markdown )
- Tera (template engine)
- Walkdir (recursively walk through directories)
- Graymatter (frontmatter parsing)
- Serde (serialize and deserialize data structure)
  - Serde YAML (for the configuration file)
  - Serde JSON (for the cache files)
- minify-html (provide a minified output HTML for extra optimizations set in the config YAML file)
- Image (convert static images to WebP for a better file size)
- Log & Env Logger (set level of verbose logging desired before run)
- Blake3 (hash file content and compare with cache to only generate a file when it has been changed)
- Which (to run an equivalent of the `which` command in Unix)

#### Styling

I recently came across TailwindCSS and figured it would the simplest way to get a decent style up for the site. My main focus was on a minimal design that is unlikely to feel outdated in the near future. The biggest reason I decided to branch out from regular CSS was to be able to pack the design easily specifying the styling through the classes Tailwind provides. That way, as the HTML is generated for the corresponding page, the classes are also generated with the corresponding styling at once, without requiring me to create my own stylesheet.

One somewhat ironic note was that I originally started developing the site with Tailwind v3 and then when I picked it back up a few months later, Tailwind v4 was introduced and caused some breaking changes on the CI/CD pipeline (I guess I haven't escaped the upgrade cycle on this site). I decided to stick with v3 and update the `package.json` script accordingly. 

### Lisp?

After using Racket extensively over a couple courses (CPSC 411 and CPSC 311 at UBC over the last year), I was thinking about making a switch over to a Lisp-based language for this project. That way, with quasi-quotation I could use S-expressions instead of worrying about a bunch of parser-related details. It could potentially look something like Markdown but with paranetheses. For example, a format I considered would have looked like the following:

```
(h1 "Blog Post 1")
(h2 "Header 2")
(p "This is a paragraph")
(code "This is a code block")
```

The nice thing about this sort of structure is that while both easier to parse thanks, it also allows for a recursive structure pretty easily and can allow for a more expressive way to provide static content. One thought was that you can run an interpreter through the content to evaluated nested expressions. For example:

```
(h1 (+ 2 2))
``` 

Where a header 1 of `(+ 2 2)` is evaluated and rendered the HTML file servered to the client. 

While a nice idea on paper, and one that I certainly wnat to explore further, I had already written a substantial portion of the SSG in Rust using Pulldown-cmark as the Markdown (or CommonMark) parser and so I decided to stick with what I currently had, leaving it as one of the few libraries allowed to use.

I was also not a huge fan if the extra boilerplate needed with the parentheseses. Sticking to regular Markdown made things a little more familiar and repeatable when writing large blog posts or new website content, and extensible with previous content on my site.  

## Performance

Performance wasn't a huge goal of mine while writing the SSG. I'm sure further optimizations could be made (and lower bundle sizes sent to users). However, given the static-nature of the site, I figured not much would be needed. 

For now, I implemented a build flag in the configuration file to minify JavaScript, CSS, and HTML files as a start. Further, I optimized each image uploaded by converting it to a `webp` file format. 

Most notably, I implemented a caching system to avoid rebuilding files that are unchanged. To do this, a timestamp is recorded from the file metadata along with the BLAKE hash function running on the contents of each file to record changes. It is saved as a `.json` file within the `build` output directory. I probably could've done this with a more performant and compact file format (using bits directly), but again, performance wasn't a major focus. 

## CI/CD

A GitHub Workflow was used to provide deployment to the GitHub Pages hosting of the site. It compiles the Rust project before running and pulls in a previous build's cache, skipping unchanged files accordingly. 