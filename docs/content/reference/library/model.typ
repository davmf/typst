#import "../../../components/index.typ": docs-category, scope

#show: docs-category.with(
  title: "Model",
  description: "Documentation for definitions related to document structure and semantics.",
  category: "model",
  groups: (
    (
      name: "diff",
      def-target: diff,
      title: "Change Tracking",
      scope: scope(std, "diff"),
      definitions: dictionary(diff),
      description: "Documentation for the `diff` module, which contains the elements that mark changes between two versions of a document.",
      docs: [
        Shows the changes between two versions of a document, similar to tracked changes in a word processor.

        Pass `--diff-base` with a git revision to `typst compile` or `typst watch` to compile the current version of a document with the changes since that revision marked:

        ```bash
        typst compile main.typ changes.pdf --diff-base HEAD~3
        ```

        The revision can be anything git understands, like `HEAD`, a branch or tag name, or a commit hash. The project files of the old version are read from the git repository that contains the project root. Packages and fonts are shared between both versions.

        By default, inserted content is green and underlined, and deleted content is red and struck through. Deleted headings and figures are neither numbered nor outlined, and labels on deleted content are removed so that references keep pointing to the current version.

        = How changes are detected <how-changes-are-detected>
        Both versions are evaluated and their content is compared, rather than their source code. Changes in a template or in code therefore show up as changes in the content they produce. Text is compared word by word. Elements with a content body, such as headings, list items, emphasis, figures, and table cells, are compared recursively if nothing but their body changed. All other elements, such as equations and images, are compared as a whole: If anything about them changed, the old version is shown as deleted and the new one as inserted.

        Changes that only affect styling, like changing the color of some text, are not marked.

        = Styling <styling>
        Changes are marked with the @diff.ins and @diff.del elements, so you can style them with show rules:

        ```example
        #show diff.ins: set text(blue)
        #show diff.del: set text(gray)
        #set strike(stroke: 1.5pt)
        The #diff.del[old]#diff.ins[new] text.
        ```

        To only show the current version with insertions highlighted, hide deleted content:

        ```example
        #show diff.del: none
        The #diff.del[old]#diff.ins[new] text.
        ```

        These definitions are part of the `diff` module and not imported by default.
      ],
    ),
  ),
)

Document structuring.

Here, you can find functions to structure your document and interact with that structure. This includes section headings, figures, bibliography management, cross-referencing and more.
