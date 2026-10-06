--- abbr paged ---
Typst exports to #abbr("Portable Document Format")[PDF].

--- abbr-show-rule paged ---
#show abbr: set text(fill: blue)
#show abbr: it => [#it.body (#it.expansion)]
Typst exports to #abbr("Portable Document Format")[PDF].

--- abbr-html html ---
Typst exports to #abbr("Portable Document Format")[PDF] and
#abbr("HyperText Markup Language")[*HTML*].

--- abbr-missing-expansion eval ---
// Error: 2-13 missing argument: body
#abbr("PDF")
