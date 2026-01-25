use pulldown_cmark::{html, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::fmt::Write;

fn replace_file_extension(file_path: &str, new_extension: &str) -> String {
    if let Some(dot_index) = file_path.rfind('.') {
        // replace the extension after the last dot
        format!("{}.{new_extension}", &file_path[..dot_index])
    } else {
        // if no extension exists append the new extension
        format!("{file_path}.{new_extension}")
    }
}

pub fn parse_markdown_with_tailwind(md_content: &str, tera: &tera::Tera) -> String {
    let parser = Parser::new_ext(md_content, Options::all());
    let mut html_output = String::new();

    let mut is_inside_header = false;
    let mut is_inside_image = false;
    let mut is_first_element = true;
    let mut image_alt_text = String::new();
    let mut current_image_src = String::new();
    let mut current_image_title = String::new();

    for event in parser {
        match event {
            // Customize headers
            Event::Start(Tag::Heading {
                level,
                id: _,
                classes: _,
                attrs: _,
            }) => {
                if is_inside_header {
                    panic!("Nested headers are not allowed");
                }
                is_inside_header = true;

                // If this is the first thing on the page (under the nav), mt should be 0.
                let mt = if is_first_element {
                    "mt-0"
                } else {
                    match level {
                        HeadingLevel::H2 => "mt-12",
                        HeadingLevel::H3 => "mt-8",
                        _ => "mt-6",
                    }
                };

                let class = match level {
                    HeadingLevel::H1 => {
                        format!("text-4xl font-extrabold text-black-600 {} mb-8", mt)
                    }
                    HeadingLevel::H2 => format!("text-3xl font-bold text-black-500 {} mb-4", mt),
                    HeadingLevel::H3 => {
                        format!("text-2xl font-semibold text-black-400 {} mb-3", mt)
                    }
                    _ => format!("text-xl font-medium text-black-300 {} mb-2", mt),
                };

                write!(html_output, "<h{} class=\"{}\">", level as u8, class).unwrap();
                is_first_element = false; // Something has been rendered
            }
            Event::End(TagEnd::Heading(level)) => {
                if is_inside_header {
                    write!(html_output, "</h{}>", level as u8).unwrap();
                    is_inside_header = false;
                }
            }
            Event::Text(text) => {
                if is_inside_image {
                    image_alt_text.push_str(&text);
                } else {
                    html_output.push_str(&text);
                }
            }

            // Image (use partial template to render)
            Event::Start(Tag::Image {
                link_type: _,
                dest_url,
                title,
                id: _,
            }) => {
                is_inside_image = true;
                image_alt_text.clear();
                current_image_src = dest_url.to_string();
                current_image_title = title.to_string();
            }

            Event::End(TagEnd::Image) => {
                let image_data = tera::Context::from_serialize(&{
                    let mut context = std::collections::HashMap::new();
                    // Match destination to the static folder within the build directory
                    // within the content markdown files DO NOT INCLUDE PATH TO static directory
                    // Just the sub-directory is needed
                    //
                    // Replace the file extension with webp (the file format that all images)
                    // turns into during the optimization/copy stages of the static folder
                    context.insert(
                        "src",
                        format!(
                            "./static/{}",
                            replace_file_extension(&current_image_src, "webp")
                        ),
                    );
                    // Use captured text from Event::Text for Alt tag
                    context.insert(
                        "alt",
                        if image_alt_text.is_empty() {
                            current_image_title.clone()
                        } else {
                            image_alt_text.clone()
                        },
                    );
                    context
                })
                .unwrap();

                // Render image using the partial template that was previously defined
                html_output.push_str(
                    &tera
                        .render("partials/image.html", &image_data)
                        .unwrap_or_else(|_| "".to_string()),
                );
                is_inside_image = false;
            }

            Event::Start(Tag::Link {
                link_type: _,
                dest_url,
                title,
                id: _,
            }) => {
                html_output.push_str(&format!(
                    "<a class=\"text-base font-bold leading-relaxed text-green-700\" href=\"{}\" title=\"{}\">",
                    dest_url.to_string(), title.to_string(),
                ));
            }

            Event::End(TagEnd::Link) => {
                html_output.push_str("</a>");
            }

            // Customize paragraphs
            Event::Start(Tag::Paragraph) => {
                html_output.push_str("<p class=\"text-base font-normal leading-relaxed mb-6\">");
            }
            Event::End(TagEnd::Paragraph) => {
                html_output.push_str("</p>");
            }

            // Customize lists
            Event::Start(Tag::List(None)) => {
                html_output.push_str(
                    "<ul class=\"list-disc text-base font-normal list-inside ml-4 mb-6\">",
                );
            }
            Event::End(TagEnd::List(_)) => {
                html_output.push_str("</ul>");
            }

            // Handle code blocks
            Event::Start(Tag::CodeBlock(kind)) => {
                let language_class = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => format!("language-{}", lang),
                    pulldown_cmark::CodeBlockKind::Indented => "language-none".to_string(),
                };
                html_output.push_str(&format!(
                    "<pre class=\"bg-gray-900 text-base font-normal text-white p-4 rounded-lg overflow-x-auto mb-6\"><code class=\"{}\">",
                    language_class
                ));
            }
            Event::End(TagEnd::CodeBlock) => {
                html_output.push_str("</code></pre>");
            }

            // Render inline code
            Event::Code(code) => {
                html_output.push_str(&format!(
                    "<code class=\"bg-gray-200 font-normal text-red-600 px-1 py-0.5 rounded\">{}</code>",
                    code
                ));
            }

            _ => {
                // Render other events normally
                html::push_html(&mut html_output, std::iter::once(event));
            }
        }
    }

    html_output
}
