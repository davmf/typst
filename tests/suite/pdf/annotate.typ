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

--- pdf-annotate-reply paged ---
#set pdf.annotate(date: datetime(year: 2026, month: 10, day: 2))
The results are #pdf.annotate(
  author: "Ana",
  "Add the error bars.",
)[significant] <bars>.

#pdf.annotate(reply-to: <bars>, author: "Ben", "Done.") <done>
#pdf.annotate(reply-to: <done>, author: "Ana", state: "accepted", "Thanks!")

--- pdf-annotate-reply-to-tooltip paged ---
#pdf.annotate(visible: false, "A tooltip")[term] <t>
#pdf.annotate(reply-to: <t>, "A reply to a tooltip.")

--- pdf-annotate-reply-body paged ---
#pdf.annotate("Note.")[text] <n>
// Error: 2-45 replies cannot have a body
#pdf.annotate(reply-to: <n>, "Reply.")[body]

--- pdf-annotate-state-without-reply paged ---
// Error: 2-46 only replies can set a review state
// Hint: 2-46 use `reply-to` to reply to an annotation
#pdf.annotate(state: "accepted", "Accepted.")

--- pdf-annotate-reply-missing paged ---
// Error: 2-45 label `<missing>` does not exist in the document
#pdf.annotate(reply-to: <missing>, "Reply.")

--- pdf-annotate-reply-not-annotation paged ---
= Heading <h>
// Error: 2-39 can only reply to `pdf.annotate`
// Hint: 2-39 the label <h> is attached to `heading`
#pdf.annotate(reply-to: <h>, "Reply.")
