#import "../../components/index.typ": docs-chapter, short-or-long

#show: docs-chapter.with(
  title: "DOCX Guide",
  route: "/guides/docx",
  description: "Learn how to export Typst documents to DOCX, collect comments and suggested changes from reviewers in Word, and bring them back into your Typst source.",
)

Typst can export documents as Word files. This is useful when your colleagues review in Word or SharePoint, but you want to keep writing in Typst. Reviewers add comments and suggest changes in the DOCX file, and Typst can then merge their feedback back into your source.

DOCX export is a one-way publishing format for review. The Typst source stays the single source of truth: you re-export after each change, rather than editing the DOCX file yourself.

= Exporting a document <exporting>
Pass a path with the `.docx` extension to the `compile` or `watch` subcommand, or choose the format explicitly with `--format docx`:

```sh
typst compile report.typ report.docx
```

Typst compiles your document the way it does for @format.html[HTML export] and converts the result to Word's format. Your templates, show rules, scripting, and packages are all evaluated as usual. Because Word, not Typst, lays out the pages, the result will not match your PDF page for page.

The following elements are converted into native Word content, so that reviewers can edit and comment on them:

- Paragraphs, headings (as Word's built-in heading styles), and text styling like bold, italic, underline, strikethrough, and highlighting
- Bulleted, numbered, and term lists
- Tables, including merged cells and repeated header rows
- Footnotes, links, and cross-references
- Equations, as native Word equations
- Images; SVG images are embedded together with a PNG fallback for older versions of Word
- Block quotes and raw text with syntax highlighting

A @grid is exported as a table without borders, so that its text stays editable. Anything else that cannot be expressed in a Word document, such as shapes and other drawings, is laid out by Typst and embedded as an image. Reviewers can comment on these images, but not edit their contents.

= Limitations <limitations>
DOCX export is experimental. The generated file has these limitations:

- The page is always A4 with 2.5 cm margins, and fonts and styles are Word's defaults. Page setup, fonts, and text colors and sizes from your Typst document are mostly not carried over.
- Numbering patterns of enumerations and headings are shown as Word's regular numbering, or as plain text.
- Headers, footers, columns, and page numbering are not exported.
- Aligned multi-line equations may not display as intended in viewers other than Word.

= Controlling what reviewers can do <protection>
By default, the exported file is protected so that every edit a reviewer makes is recorded as a tracked change. Reviewers can still add comments. Use `--docx-protection` to choose another level:

#table(
  columns: (auto, 1fr),
  [*Value*], [*Reviewers can \...*],
  [`tracked-changes`], [comment, and edit with all edits recorded as tracked changes (default)],
  [`comments`], [only comment],
  [`read-only`], [only read the document],
  [`none`], [edit the document freely],
)

```sh
typst compile report.typ report.docx --docx-protection comments
```

The protection is written into the file without a password. It guards against accidental edits, but anyone can turn it off in Word, and anyone who can download the file can save an unprotected copy. If reviewers must not edit the document, additionally restrict their access where you share the file. On SharePoint and OneDrive, share the file with the "Can review" permission. This puts the file into suggesting mode for reviewers and is enforced by the service.

= Comments in the exported file <comments>
Notes you create with @pdf.annotate[`pdf.annotate`] become Word comments in the exported file, with their author and date. Replies, created with `reply-to`, become replies in the same comment thread, and a reply that sets `state` to `{"completed"}` resolves the thread:

```typ
#set pdf.annotate(author: "Ben")

The method is #pdf.annotate("Cite the original paper.")[novel] <novel>.

#pdf.annotate(reply-to: <novel>, author: "Ana", "Which one?")
```

If you compile with `--diff-base`, the changes since the given git revision are exported as tracked changes by the author "Typst". Reviewers see exactly what changed since the last round, and `typst docx-import` ignores these changes. See the @diff[change tracking] documentation for details.

= Bringing feedback back into your source <importing>
After the review, run `typst docx-import` with your source file and the reviewed DOCX file:

```sh
typst docx-import report.typ report-reviewed.docx --dry-run
typst docx-import report.typ report-reviewed.docx
```

With `--dry-run`, Typst only prints what it found and which edits it would make. Without it, the edits are written to your source files.

Typst finds each comment and change by comparing the text of the reviewed file with a fresh export of your current source. It does not rely on anything stored in the file by Typst, so this also works if Word has rewritten the file or if you edited the source slightly since you exported it. The import then does the following:

- New comments become `pdf.annotate` calls around the commented text. If the text cannot be wrapped, for example because it spans several paragraphs, the comment becomes a note at the start of the text instead.
- Replies become `pdf.annotate` calls with `reply-to`, and resolved threads get a reply with `state: "completed"`.
- Tracked changes to plain text are applied to your source. Changes that cannot be applied safely, such as those inside equations or across markup, become notes that describe the suggested change.
- Comments that are already in your source are skipped, so importing the same file twice does not create duplicates.

Use `--changes` to choose what happens with tracked changes: `apply` (the default) edits the text, `annotate` turns all changes into notes and leaves your text as it is, and `ignore` only lists them.

After the import, review the result with your version control system and the annotations in your document. Delete the annotations once you have dealt with them, and export a fresh DOCX file for the next round.

= Workflow for review rounds <workflow>
A typical review round looks like this:

+ Export the document with `typst compile report.typ report.docx` and share it with your reviewers, for example with the "Can review" permission on SharePoint.
+ Reviewers add comments and suggested changes in Word.
+ Download the reviewed file and run `typst docx-import report.typ report-reviewed.docx`.
+ Go through the annotations and changes in your source, commit them, and resolve the comments.
+ For the next round, export again with `--diff-base` set to the revision of the previous round, so reviewers can see what changed.
