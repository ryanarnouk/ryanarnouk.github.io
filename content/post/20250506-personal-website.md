---
title: "Remaking My Personal Website"
date: "05-06-2025"
tags: ["Rust", "Web Development"]
---

An overview of my website, bundled with a static site generator written in Rust.

## Motivation

I want to develop a robust and solid site without worrying about maintenance and package upgrades for the foreseeable future. I often neglect to keep my tech stacks up to date on my previous sites. 

My old personal pages usually rely on too many tools, and the pace of development and upgrade cycles outpaces the amount of time I am able to spend maintaining them. I end up with old projects with terribly old code bases that need upgrading.

It was sometimes even easier to settle for remaking my page altogether. My hope with offering a greater level of control over the SSG in this version of my site is that this will not need to happen as often.

## Static Site Generator

I originally intended to write the static site generator component of the project in OCaml, but later settled for Rust for familiarity and libraries that are available in the Rust ecosystem.

However, the expansive ecosystem of Rust libraries presents a challenge of where to strike the balance between libraries (and other dependencies) and my goal of having a simplistic website that has very little reliance on code that will constantly be updated and may have vulnerabilities. On the other hand, writing everything myself would have been way too long (writing the parser for markdown alone would have taken a while).

Because of this, I settled for a balance. While I could use libraries, most of the system would be well understood by me to make updates later on in the future (and make dependency upgrades as needed). And, nothing is using "flashy" web development technologies that come and go (except for TailwindCSS, arguably).

### Libraries

To avoid my issues using web development frameworks and libraries in the past, and falling behind in upgrades, I tried to keep the libraries needed for this project to a minimum. Most of which probably wouldn't require any ground-breaking changes in the near future (I hope).

In total, the following libraries were used (installed via Cargo):
- Pulldown-cmark (parser for Markdown )
- Tera (template engine)
- Walkdir (recursively walk through directories)
- Graymatter (frontmatter parsing)
- Serde (serialize and deserialize data structure)
  - Serde YAML (for the configuration file)
  - Serde JSON (for the cache files)
- minify-html (provide a minified output HTML for extra optimizations set in the config YAML file)
- Image (convert static images to WebP for a better file size)
- Log & Env Logger (set the level of verbose logging desired before running)
- Blake3 (hash file content and compare with cache to only generate a file when it has been changed)
- Which (to run an equivalent of the `which` command in Unix)

#### Styling

I recently came across TailwindCSS and figured it would be the simplest way to get a decent style up for the site. My main focus was on a minimal design that is unlikely to feel outdated shortly. The biggest reason I decided to branch out from regular CSS was to be able to pack the design easily, specifying the styling through the classes Tailwind provides. That way, as the HTML is generated for the corresponding page, the classes are also generated with the corresponding styling at once, without requiring me to create my stylesheet.

One somewhat ironic note was that I originally started developing the site with Tailwind v3, and then when I picked it back up a few months later, Tailwind v4 was introduced and caused some breaking changes on the CI/CD pipeline (I guess I haven't escaped the upgrade cycle on this site). I decided to stick with v3 and update the `package.json` script accordingly. 

### Lisp?

After using Racket extensively over a couple of courses (CPSC 311 and 411 at UBC), I was thinking about making a switch to a Lisp-based language for this project. That way, with quasi-quotation for pattern matching, I could use [S-expressions](https://en.wikipedia.org/wiki/S-expression) instead of worrying about a bunch of parser-related details. A brief introduction to quasi-quotation in Lisp-based languages can be found [here](https://en.wikipedia.org/wiki/Lisp_(programming_language)#Self-evaluating_forms_and_quoting). It could potentially look something like Markdown but with parentheses. For example, a format I considered would have looked like the following:

```
(h1 "Blog Post 1")
(h2 "Header 2")
(p "This is a paragraph")
(code "This is a code block")
```

The nice thing about this sort of structure is that while both are easier to parse, it also allows for a recursive structure pretty easily and can allow for a more expressive way to provide static content. One thought was that you can run an interpreter through the content and evaluate nested expressions before rendering. For example:

```
(h1 (+ 2 2))
``` 

Where a header 1 of `(+ 2 2)` would be evaluated and rendered as "4" in the HTML file served to the client. 

While a nice idea on paper, and one that I certainly want to explore further, I had already written a substantial portion of the SSG in Rust using Pulldown-cmark as the Markdown (or CommonMark) parser and so I decided to stick with what I currently had, leaving it as one of the few libraries allowed to use.

I was also not a huge fan of the extra boilerplate needed with the parentheses. Sticking to regular Markdown made things a little more familiar and repeatable when writing large blog posts or new website content, and extensible with previous content on my site.  

## Performance

Performance wasn't a huge goal of mine while writing the SSG. I'm sure further optimizations could be made (and lower bundle sizes sent to users). However, given the static nature of the site, I figured not much would be needed. 

For now, I implemented a build flag in the configuration file to minify JavaScript, CSS, and HTML files as a start. Further, I optimized each image uploaded by converting it to a `webp` file format. 

Most notably, I implemented a caching system to avoid rebuilding files that are unchanged. To do this, a timestamp is recorded from the file metadata along with the BLAKE hash function running on the contents of each file to record changes. It is saved as a `.json` file within the `build` output directory. I probably could've done this with a more performant and compact file format (maybe storing as a binary), but again, performance wasn't a major focus. 

## CI/CD

A GitHub Workflow was used to provide deployment to the GitHub Pages hosting of the site. It compiles the Rust project before running and pulls in a previous build's cache, skipping unchanged files accordingly. 