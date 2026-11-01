use pulldown_cmark::{
    html, CodeBlockKind, CowStr, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd,
};

use crate::resources;

// Escape text for use inside a double-quoted HTML attribute
fn escape_attr(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

// Absolute URLs (and root-relative paths) are left untouched. Anything else is treated as a path
// inside the static folder (content markdown files DO NOT INCLUDE the path to the static
// directory, just the sub-directory) and mapped to the file name produced by the static
// resource pipeline (e.g. PNGs are converted to WebP)
fn resolve_image_src(src: &str) -> String {
    if src.contains("://") || src.starts_with('/') || src.starts_with("data:") {
        src.to_string()
    } else {
        let relative = src.trim_start_matches("./");
        format!("./static/{}", resources::output_file_name(relative))
    }
}

fn html_event(html: String) -> Event<'static> {
    Event::Html(CowStr::from(html))
}

pub fn parse_markdown_with_tailwind(
    md_content: &str,
    tera: &tera::Tera,
) -> Result<String, tera::Error> {
    let parser = Parser::new_ext(md_content, Options::all());
    let mut events: Vec<Event> = Vec::new();

    // Only the first heading gets a top margin of 0 (see the heading handling below)
    let mut is_first_heading = true;
    // Some((src, title, alt text)) while inside an image
    let mut current_image: Option<(String, String, String)> = None;

    for event in parser {
        // Text inside an image becomes the alt text instead of being rendered
        if let Some((src, title, alt)) = current_image.as_mut() {
            match event {
                Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                Event::End(TagEnd::Image) => {
                    let mut context = tera::Context::new();
                    context.insert("src", &escape_attr(&resolve_image_src(src)));
                    context.insert("alt", if alt.is_empty() { &*title } else { &*alt });

                    // Render image using the partial template that was previously defined
                    events.push(html_event(tera.render("partials/image.html", &context)?));
                    current_image = None;
                }
                _ => {}
            }
            continue;
        }

        let event = match event {
            // Customize headers
            Event::Start(Tag::Heading { level, .. }) => {
                // The first heading on the page (under the nav) gets mt-0.
                // NOTE: this applies even when other content (paragraphs, images) comes before it
                let mt = if is_first_heading {
                    "mt-0"
                } else {
                    match level {
                        HeadingLevel::H2 => "mt-12",
                        HeadingLevel::H3 => "mt-8",
                        _ => "mt-6",
                    }
                };

                let class = match level {
                    HeadingLevel::H1 => format!("text-4xl font-extrabold text-black-600 {mt} mb-8"),
                    HeadingLevel::H2 => format!("text-3xl font-bold text-black-500 {mt} mb-4"),
                    HeadingLevel::H3 => format!("text-2xl font-semibold text-black-400 {mt} mb-3"),
                    _ => format!("text-xl font-medium text-black-300 {mt} mb-2"),
                };

                is_first_heading = false;
                html_event(format!("<{level} class=\"{class}\">"))
            }
            Event::End(TagEnd::Heading(level)) => html_event(format!("</{level}>")),

            // Image (use partial template to render once the alt text has been collected)
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                current_image = Some((dest_url.to_string(), title.to_string(), String::new()));
                continue;
            }

            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }) => {
                let href = if link_type == LinkType::Email {
                    format!("mailto:{dest_url}")
                } else {
                    dest_url.to_string()
                };
                let title = if title.is_empty() {
                    String::new()
                } else {
                    format!(" title=\"{}\"", escape_attr(&title))
                };
                html_event(format!(
                    "<a class=\"text-base font-bold leading-relaxed text-green-700\" href=\"{}\"{}>",
                    escape_attr(&href),
                    title
                ))
            }
            Event::End(TagEnd::Link) => html_event("</a>".to_string()),

            // Customize paragraphs
            Event::Start(Tag::Paragraph) => {
                html_event("<p class=\"text-base font-normal leading-relaxed mb-6\">".to_string())
            }
            Event::End(TagEnd::Paragraph) => html_event("</p>".to_string()),

            // Customize lists
            Event::Start(Tag::List(None)) => html_event(
                "<ul class=\"list-disc text-base font-normal list-inside ml-4 mb-6\">".to_string(),
            ),
            Event::Start(Tag::List(Some(start))) => {
                let start = if start == 1 {
                    String::new()
                } else {
                    format!(" start=\"{start}\"")
                };
                html_event(format!(
                    "<ol class=\"list-decimal text-base font-normal list-inside ml-4 mb-6\"{start}>"
                ))
            }
            Event::End(TagEnd::List(ordered)) => {
                html_event(if ordered { "</ol>" } else { "</ul>" }.to_string())
            }

            // Handle code blocks (the code itself arrives as Text events, escaped by push_html)
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(lang) if !lang.is_empty() => lang.to_string(),
                    _ => "none".to_string(),
                };
                html_event(format!(
                    "<pre class=\"bg-gray-900 text-base font-normal text-white p-4 rounded-lg overflow-x-auto mb-6\"><code class=\"language-{}\">",
                    escape_attr(&language)
                ))
            }
            Event::End(TagEnd::CodeBlock) => html_event("</code></pre>".to_string()),

            // Render inline code
            Event::Code(code) => Event::InlineHtml(CowStr::from(format!(
                "<code class=\"bg-gray-200 font-normal text-red-600 px-1 py-0.5 rounded\">{}</code>",
                escape_attr(&code)
            ))),

            // Render other events normally
            other => other,
        };
        events.push(event);
    }

    // Render everything in a single pass so pulldown-cmark can track state across events (table
    // headers, footnotes, ...) and escape text correctly
    let mut html_output = String::with_capacity(md_content.len() * 3 / 2);
    html::push_html(&mut html_output, events.into_iter());
    Ok(html_output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(md: &str) -> String {
        let mut tera = tera::Tera::default();
        tera.add_raw_template(
            "partials/image.html",
            "<img src=\"{{ src | safe }}\" alt=\"{{ alt }}\">",
        )
        .unwrap();
        parse_markdown_with_tailwind(md, &tera).unwrap()
    }

    #[test]
    fn escapes_code_and_text() {
        let html = render("```html\n<div>\n```\n\n`<b>` and 1 < 2");
        assert!(html.contains("&lt;div&gt;"));
        assert!(html.contains("&lt;b&gt;"));
        assert!(html.contains("1 &lt; 2"));
        assert!(!html.contains("<div>"));
    }

    #[test]
    fn closes_ordered_lists() {
        let html = render("1. one\n2. two");
        assert!(html.contains("<ol") && html.contains("</ol>") && !html.contains("</ul>"));
    }

    #[test]
    fn email_autolinks_use_mailto() {
        assert!(render("<me@example.com>").contains("href=\"mailto:me@example.com\""));
    }

    #[test]
    fn image_sources() {
        assert!(
            render("![alt](images/a.png)").contains("src=\"./static/images/a.webp\" alt=\"alt\"")
        );
        assert!(render("![x](images/a.svg)").contains("src=\"./static/images/a.svg\""));
        assert!(render("![x](https://e.com/a.png)").contains("src=\"https://e.com/a.png\""));
    }

    #[test]
    fn escape_attr_escapes_quotes_and_markup() {
        assert_eq!(
            escape_attr(r#"a "b" <c> & d"#),
            "a &quot;b&quot; &lt;c&gt; &amp; d"
        );
        assert_eq!(escape_attr("plain"), "plain");
    }

    #[test]
    fn styles_paragraphs_and_lists() {
        let html = render("text\n\n- a\n- b");
        assert!(html.contains("<p class=\"text-base font-normal leading-relaxed mb-6\">text</p>"));
        assert!(html.contains("<ul class=\"list-disc"));
        assert!(html.contains("<li>a</li>"));
        assert!(html.ends_with("</ul>\n") || html.ends_with("</ul>"));
    }

    #[test]
    fn ordered_lists_keep_their_start_number() {
        assert!(render("3. three\n4. four").contains("start=\"3\""));
        assert!(!render("1. one").contains("start="));
    }

    #[test]
    fn nested_lists_close_correctly() {
        let html = render("1. one\n   - a\n   - b\n2. two");
        assert_eq!(html.matches("<ol").count(), html.matches("</ol>").count());
        assert_eq!(html.matches("<ul").count(), html.matches("</ul>").count());
    }

    #[test]
    fn heading_levels_get_their_classes() {
        let html = render("# A\n\n## B\n\n### C\n\n#### D");
        assert!(
            html.contains("<h1 class=\"text-4xl font-extrabold text-black-600 mt-0 mb-8\">A</h1>")
        );
        assert!(html.contains("<h2 class=\"text-3xl font-bold text-black-500 mt-12 mb-4\">B</h2>"));
        assert!(
            html.contains("<h3 class=\"text-2xl font-semibold text-black-400 mt-8 mb-3\">C</h3>")
        );
        assert!(html.contains("<h4 class=\"text-xl font-medium text-black-300 mt-6 mb-2\">D</h4>"));
    }

    #[test]
    fn heading_text_is_escaped() {
        let html = render("# a < b & c");
        assert!(html.contains(">a &lt; b &amp; c</h1>"), "{html}");
    }

    #[test]
    fn links_are_styled_and_escaped() {
        let html = render("[x](https://e.com/?a=1&b=\"2\" \"A \\\"title\\\"\")");
        assert!(
            html.contains("class=\"text-base font-bold leading-relaxed text-green-700\""),
            "{html}"
        );
        assert!(html.contains("&amp;b="), "{html}");
        assert!(html.contains("title=\"A &quot;title&quot;\""), "{html}");
        assert!(!render("[x](/a.html)").contains("title="));
    }

    #[test]
    fn code_blocks_get_a_language_class() {
        assert!(render("```rust\nfn main() {}\n```")
            .contains("<code class=\"language-rust\">fn main() {}"));
        assert!(render("```\nplain\n```").contains("<code class=\"language-none\">"));
        assert!(render("    indented").contains("<code class=\"language-none\">"));
    }

    #[test]
    fn image_alt_text_falls_back_to_title() {
        assert!(render("![](a.png \"The title\")").contains("alt=\"The title\""));
        assert!(render("![Alt `code`](a.png \"t\")").contains("alt=\"Alt code\""));
    }

    #[test]
    fn image_alt_text_is_escaped() {
        // Straight quotes become curly ones (smart punctuation is enabled), so only & and < remain
        let html = render("![a & 1 < 2](a.png)");
        assert!(html.contains("alt=\"a &amp; 1 &lt; 2\""), "{html}");
    }

    #[test]
    fn image_sources_relative_to_static_folder() {
        assert!(render("![x](./images/a.jpg)").contains("src=\"./static/images/a.webp\""));
        assert!(render("![x](/images/a.png)").contains("src=\"/images/a.png\""));
    }

    #[test]
    fn tables_render_header_cells() {
        let html = render("| A | B |\n|---|---|\n| 1 | 2 |");
        assert!(html.contains("<th>A</th>"), "{html}");
        assert!(html.contains("<td>1</td>"), "{html}");
    }

    #[test]
    fn missing_image_partial_is_an_error() {
        let tera = tera::Tera::default();
        assert!(parse_markdown_with_tailwind("![x](a.png)", &tera).is_err());
        assert!(parse_markdown_with_tailwind("no images", &tera).is_ok());
    }

    #[test]
    fn only_first_heading_has_no_top_margin() {
        let html = render("## A\n\n## B");
        assert!(html.contains("mt-0") && html.contains("mt-12"));
    }
}
