// Test PDF annotations.

--- pdf-annotate-note paged ---
#set pdf.annotate(author: "Ana")
The results are #pdf.annotate("Add the error bars.")[significant].
A note without body: #pdf.annotate("Expand this.", icon: "note", color: blue)

--- pdf-annotate-note-properties paged ---
#pdf.annotate(
  "Half transparent.",
  subject: "Style",
  open: true,
  opacity: 50%,
  date: datetime(year: 2026, month: 10, day: 2),
)[faded]

--- pdf-annotate-invisible paged ---
// Invisible notes don't affect the layout.
#set page(width: 120pt)
#pdf.annotate(visible: false, "A tooltip")[A tooltip that is long enough to wrap].

--- pdf-annotate-hidden paged ---
#hide[#pdf.annotate("Not exported.")[hidden]]

--- pdf-annotate-nested paged ---
#pdf.annotate(visible: false, "Outer")[outer #pdf.annotate("Inner")[inner] text]

--- pdf-annotate-link paged ---
#pdf.annotate("Check the URL.")[#link("https://typst.app")[typst.app]]

--- pdf-annotate-tooltip paged ---
#tooltip("Portable Document Format")[PDF]

--- pdf-annotate-html html ---
#pdf.annotate("Not shown in HTML.")[body]
#pdf.annotate("Not shown in HTML.")

--- pdf-annotate-invalid-icon eval ---
// Error: 34-40 expected "comment", "note", "help", "key", "insert", "paragraph", or "new-paragraph"
#pdf.annotate("Contents.", icon: "star")
