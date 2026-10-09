//! Assembly of the DOCX package.

use std::fmt::Write as _;
use std::io::Write as _;

use typst_html::HtmlDocument;
use typst_library::model::Document;

use crate::write::{ListFormat, Output};
use crate::xml::{DECLARATION, NAMESPACES, escape, escape_into};
use crate::{DocxOptions, Protection};

/// Builds the DOCX file from the converted document.
pub fn package(
    output: &Output,
    document: &HtmlDocument,
    options: &DocxOptions,
) -> Vec<u8> {
    let mut parts: Vec<(String, Vec<u8>)> = vec![];
    let mut add = |name: &str, data: String| parts.push((name.into(), data.into_bytes()));

    let has_lists = !output.lists.is_empty();
    let has_footnotes = !output.footnotes.is_empty();
    let has_comments = !output.comments.is_empty();

    add("[Content_Types].xml", content_types(has_lists, has_footnotes, has_comments));
    add("_rels/.rels", ROOT_RELS.into());
    add("docProps/core.xml", core_props(document, options));
    add("docProps/app.xml", APP_PROPS.into());
    add(
        "word/_rels/document.xml.rels",
        document_rels(output, has_lists, has_footnotes, has_comments),
    );
    add("word/document.xml", document_xml(output));
    add("word/styles.xml", styles(document));
    add("word/settings.xml", settings(options));
    if has_lists {
        add("word/numbering.xml", numbering(&output.lists));
    }
    if has_footnotes {
        add("word/footnotes.xml", footnotes(&output.footnotes));
    }
    if has_comments {
        add("word/comments.xml", comments(output));
        add("word/commentsExtended.xml", comments_extended(output));
    }
    for media in &output.media {
        parts.push((format!("word/media/{}", media.name), media.data.clone()));
    }

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in parts {
            zip.start_file(name, opts).unwrap();
            zip.write_all(&data).unwrap();
        }
        zip.finish().unwrap();
    }
    buffer.into_inner()
}

const ROOT_RELS: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>",
    "<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>",
    "<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>",
    "</Relationships>",
);

const APP_PROPS: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\">",
    "<Application>Typst</Application></Properties>",
);

fn content_types(lists: bool, footnotes: bool, comments: bool) -> String {
    let mut out = String::from(DECLARATION);
    out.push_str(
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Default Extension=\"png\" ContentType=\"image/png\"/>\
         <Default Extension=\"jpg\" ContentType=\"image/jpeg\"/>\
         <Default Extension=\"gif\" ContentType=\"image/gif\"/>\
         <Default Extension=\"svg\" ContentType=\"image/svg+xml\"/>",
    );
    let wml = "application/vnd.openxmlformats-officedocument.wordprocessingml";
    let mut over = |part: &str, ty: &str| {
        write!(out, "<Override PartName=\"{part}\" ContentType=\"{ty}\"/>").unwrap();
    };
    over("/word/document.xml", &format!("{wml}.document.main+xml"));
    over("/word/styles.xml", &format!("{wml}.styles+xml"));
    over("/word/settings.xml", &format!("{wml}.settings+xml"));
    if lists {
        over("/word/numbering.xml", &format!("{wml}.numbering+xml"));
    }
    if footnotes {
        over("/word/footnotes.xml", &format!("{wml}.footnotes+xml"));
    }
    if comments {
        over("/word/comments.xml", &format!("{wml}.comments+xml"));
        over("/word/commentsExtended.xml", &format!("{wml}.commentsExtended+xml"));
    }
    over(
        "/docProps/core.xml",
        "application/vnd.openxmlformats-package.core-properties+xml",
    );
    over(
        "/docProps/app.xml",
        "application/vnd.openxmlformats-officedocument.extended-properties+xml",
    );
    out.push_str("</Types>");
    out
}

fn document_rels(
    output: &Output,
    lists: bool,
    footnotes: bool,
    comments: bool,
) -> String {
    let base = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let mut out = String::from(DECLARATION);
    out.push_str("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
    let mut rel = |id: &str, ty: &str, target: &str, external: bool| {
        write!(
            out,
            "<Relationship Id=\"{id}\" Type=\"{ty}\" Target=\"{}\"",
            escape(target)
        )
        .unwrap();
        if external {
            out.push_str(" TargetMode=\"External\"");
        }
        out.push_str("/>");
    };
    rel("rId1", &format!("{base}/styles"), "styles.xml", false);
    rel("rId2", &format!("{base}/settings"), "settings.xml", false);
    if lists {
        rel("rId3", &format!("{base}/numbering"), "numbering.xml", false);
    }
    if footnotes {
        rel("rId4", &format!("{base}/footnotes"), "footnotes.xml", false);
    }
    if comments {
        rel("rId5", &format!("{base}/comments"), "comments.xml", false);
        rel(
            "rId6",
            "http://schemas.microsoft.com/office/2011/relationships/commentsExtended",
            "commentsExtended.xml",
            false,
        );
    }
    for r in &output.rels {
        rel(&r.id, r.kind, &r.target, r.external);
    }
    out.push_str("</Relationships>");
    out
}

fn document_xml(output: &Output) -> String {
    let mut out = String::from(DECLARATION);
    write!(out, "<w:document {NAMESPACES}><w:body>").unwrap();
    out.push_str(&output.body);
    // An A4 page with 2.5cm margins, matching the frame layout width.
    out.push_str(
        "<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
         <w:pgMar w:top=\"1417\" w:right=\"1417\" w:bottom=\"1417\" w:left=\"1417\" \
         w:header=\"709\" w:footer=\"709\" w:gutter=\"0\"/></w:sectPr>",
    );
    out.push_str("</w:body></w:document>");
    out
}

fn core_props(document: &HtmlDocument, options: &DocxOptions) -> String {
    let info = document.info();
    let mut out = String::from(DECLARATION);
    out.push_str(
        "<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
         xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">",
    );
    if let Some(title) = &info.title {
        write!(out, "<dc:title>{}</dc:title>", escape(title)).unwrap();
    }
    if !info.author.is_empty() {
        write!(out, "<dc:creator>{}</dc:creator>", escape(&info.author.join(", ")))
            .unwrap();
    }
    if let Some(description) = &info.description {
        write!(out, "<dc:description>{}</dc:description>", escape(description)).unwrap();
    }
    if !info.keywords.is_empty() {
        write!(out, "<cp:keywords>{}</cp:keywords>", escape(&info.keywords.join(", ")))
            .unwrap();
    }
    if let Some(timestamp) = &options.timestamp {
        write!(
            out,
            "<dcterms:created xsi:type=\"dcterms:W3CDTF\">{0}</dcterms:created>\
             <dcterms:modified xsi:type=\"dcterms:W3CDTF\">{0}</dcterms:modified>",
            escape(timestamp)
        )
        .unwrap();
    }
    out.push_str("</cp:coreProperties>");
    out
}

fn settings(options: &DocxOptions) -> String {
    let mut out = String::from(DECLARATION);
    write!(out, "<w:settings {NAMESPACES}><w:zoom w:percent=\"100\"/>").unwrap();
    let edit = match options.protection {
        Protection::None => None,
        Protection::Comments => Some("comments"),
        Protection::TrackedChanges => Some("trackedChanges"),
        Protection::ReadOnly => Some("readOnly"),
    };
    if options.protection == Protection::TrackedChanges {
        out.push_str("<w:trackRevisions/>");
    }
    if let Some(edit) = edit {
        write!(out, "<w:documentProtection w:edit=\"{edit}\" w:enforcement=\"1\"/>")
            .unwrap();
    }
    out.push_str(
        "<w:defaultTabStop w:val=\"720\"/>\
         <w:characterSpacingControl w:val=\"doNotCompress\"/>\
         <w:compat><w:compatSetting w:name=\"compatibilityMode\" \
         w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat>\
         </w:settings>",
    );
    out
}

fn numbering(lists: &[ListFormat]) -> String {
    let mut out = String::from(DECLARATION);
    write!(out, "<w:numbering {NAMESPACES}>").unwrap();
    for (i, list) in lists.iter().enumerate() {
        write!(out, "<w:abstractNum w:abstractNumId=\"{i}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>").unwrap();
        for level in 0..9 {
            let left = 720 * (level + 1);
            let (fmt, text, start) = match list {
                ListFormat::Bullet => {
                    let bullet = ["•", "◦", "▪"][level % 3];
                    ("bullet", bullet.to_string(), 1)
                }
                ListFormat::Ordered { start } => {
                    let fmt = ["decimal", "lowerLetter", "lowerRoman"][level % 3];
                    (fmt, format!("%{}.", level + 1), if level == 0 { *start } else { 1 })
                }
            };
            write!(
                out,
                "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"{start}\"/>\
                 <w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{text}\"/><w:lvlJc w:val=\"left\"/>\
                 <w:pPr><w:ind w:left=\"{left}\" w:hanging=\"360\"/></w:pPr></w:lvl>"
            )
            .unwrap();
        }
        out.push_str("</w:abstractNum>");
    }
    for i in 0..lists.len() {
        write!(
            out,
            "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{i}\"/></w:num>",
            i + 1
        )
        .unwrap();
    }
    out.push_str("</w:numbering>");
    out
}

fn footnotes(notes: &[String]) -> String {
    let mut out = String::from(DECLARATION);
    write!(out, "<w:footnotes {NAMESPACES}>").unwrap();
    out.push_str(
        "<w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:pPr><w:spacing w:after=\"0\" \
         w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:r><w:separator/></w:r></w:p></w:footnote>\
         <w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:pPr><w:spacing \
         w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:r><w:continuationSeparator/>\
         </w:r></w:p></w:footnote>",
    );
    for (i, note) in notes.iter().enumerate() {
        write!(out, "<w:footnote w:id=\"{}\">{note}</w:footnote>", i + 1).unwrap();
    }
    out.push_str("</w:footnotes>");
    out
}

fn initials(author: &str) -> String {
    author.split_whitespace().filter_map(|w| w.chars().next()).collect()
}

fn comments(output: &Output) -> String {
    let mut out = String::from(DECLARATION);
    write!(out, "<w:comments {NAMESPACES}>").unwrap();
    for comment in &output.comments {
        write!(
            out,
            "<w:comment w:id=\"{}\" w:author=\"{}\" w:initials=\"{}\"",
            comment.id,
            escape(&comment.author),
            escape(&initials(&comment.author)),
        )
        .unwrap();
        if let Some(date) = &comment.date {
            write!(out, " w:date=\"{}\"", escape(date)).unwrap();
        }
        write!(
            out,
            "><w:p w14:paraId=\"{:08X}\" w14:textId=\"77777777\"><w:pPr><w:pStyle \
             w:val=\"CommentText\"/></w:pPr><w:r><w:rPr><w:rStyle w:val=\"CommentReference\"/>\
             </w:rPr><w:annotationRef/></w:r><w:r>",
            comment.para_id
        )
        .unwrap();
        for (i, line) in comment.text.split('\n').enumerate() {
            if i > 0 {
                out.push_str("<w:br/>");
            }
            out.push_str("<w:t xml:space=\"preserve\">");
            escape_into(&mut out, line);
            out.push_str("</w:t>");
        }
        out.push_str("</w:r></w:p></w:comment>");
    }
    out.push_str("</w:comments>");
    out
}

fn comments_extended(output: &Output) -> String {
    let mut out = String::from(DECLARATION);
    write!(out, "<w15:commentsEx {NAMESPACES}>").unwrap();
    for comment in &output.comments {
        write!(out, "<w15:commentEx w15:paraId=\"{:08X}\"", comment.para_id).unwrap();
        if let Some(parent) = comment.parent {
            write!(out, " w15:paraIdParent=\"{parent:08X}\"").unwrap();
        }
        write!(out, " w15:done=\"{}\"/>", u8::from(comment.done)).unwrap();
    }
    out.push_str("</w15:commentsEx>");
    out
}

fn styles(document: &HtmlDocument) -> String {
    let lang = document
        .root()
        .attrs
        .get(typst_html::attr::lang)
        .map(|l| l.to_string())
        .unwrap_or_else(|| "en-US".into());

    let mut out = String::from(DECLARATION);
    write!(out, "<w:styles {NAMESPACES}>").unwrap();
    write!(
        out,
        "<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" \
         w:eastAsia=\"Calibri\" w:cs=\"Calibri\"/><w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/>\
         <w:lang w:val=\"{}\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr>\
         <w:spacing w:after=\"160\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault>\
         </w:docDefaults>",
        escape(&lang)
    )
    .unwrap();

    out.push_str(
        "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>",
    );
    let para = |out: &mut String, id: &str, name: &str, ppr: &str, rpr: &str| {
        write!(
            out,
            "<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/>\
             <w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/>\
             <w:pPr>{ppr}</w:pPr><w:rPr>{rpr}</w:rPr></w:style>"
        )
        .unwrap();
    };
    para(
        &mut out,
        "Title",
        "Title",
        "<w:spacing w:after=\"240\"/><w:jc w:val=\"center\"/>",
        "<w:b/><w:sz w:val=\"48\"/><w:szCs w:val=\"48\"/>",
    );
    let sizes = [32, 28, 26, 24, 22, 22, 22, 22, 22];
    for (i, size) in sizes.iter().enumerate() {
        para(
            &mut out,
            &format!("Heading{}", i + 1),
            &format!("heading {}", i + 1),
            &format!(
                "<w:keepNext/><w:keepLines/><w:spacing w:before=\"{}\" w:after=\"120\"/>\
                 <w:outlineLvl w:val=\"{i}\"/>",
                if i == 0 { 360 } else { 240 }
            ),
            &format!("<w:b/><w:bCs/><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/>"),
        );
    }
    para(
        &mut out,
        "Caption",
        "caption",
        "<w:jc w:val=\"center\"/><w:spacing w:after=\"240\"/>",
        "<w:i/><w:sz w:val=\"20\"/>",
    );
    para(&mut out, "Quote", "Quote", "<w:ind w:left=\"720\" w:right=\"720\"/>", "<w:i/>");
    para(
        &mut out,
        "SourceCode",
        "Source Code",
        "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F3F3F3\"/><w:spacing w:after=\"160\" w:line=\"240\" w:lineRule=\"auto\"/>",
        "<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/><w:sz w:val=\"19\"/>",
    );
    para(
        &mut out,
        "ListParagraph",
        "List Paragraph",
        "<w:spacing w:after=\"60\"/><w:contextualSpacing/>",
        "",
    );
    para(
        &mut out,
        "DefinitionTerm",
        "Definition Term",
        "<w:keepNext/><w:spacing w:after=\"0\"/>",
        "<w:b/>",
    );
    para(&mut out, "Definition", "Definition", "<w:ind w:left=\"720\"/>", "");
    para(
        &mut out,
        "FootnoteText",
        "footnote text",
        "<w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/>",
        "<w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/>",
    );
    para(
        &mut out,
        "CommentText",
        "annotation text",
        "<w:spacing w:line=\"240\" w:lineRule=\"auto\"/>",
        "<w:sz w:val=\"20\"/>",
    );

    let chr = |out: &mut String, id: &str, name: &str, rpr: &str| {
        write!(
            out,
            "<w:style w:type=\"character\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/>\
             <w:rPr>{rpr}</w:rPr></w:style>"
        )
        .unwrap();
    };
    chr(
        &mut out,
        "VerbatimChar",
        "Verbatim Char",
        "<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/><w:sz w:val=\"20\"/>",
    );
    chr(
        &mut out,
        "Hyperlink",
        "Hyperlink",
        "<w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/>",
    );
    chr(
        &mut out,
        "FootnoteReference",
        "footnote reference",
        "<w:vertAlign w:val=\"superscript\"/>",
    );
    chr(&mut out, "CommentReference", "annotation reference", "<w:sz w:val=\"16\"/>");

    out.push_str(
        "<w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\"><w:name w:val=\"Normal Table\"/>\
         <w:tblPr><w:tblInd w:w=\"0\" w:type=\"dxa\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/>\
         <w:left w:w=\"108\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"108\" w:type=\"dxa\"/>\
         </w:tblCellMar></w:tblPr></w:style>\
         <w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/>\
         <w:basedOn w:val=\"TableNormal\"/><w:pPr><w:spacing w:before=\"40\" w:after=\"40\"/></w:pPr>\
         <w:tblPr><w:tblBorders>\
         <w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         <w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         <w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         <w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         <w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         <w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         </w:tblBorders></w:tblPr></w:style>",
    );
    out.push_str("</w:styles>");
    out
}
