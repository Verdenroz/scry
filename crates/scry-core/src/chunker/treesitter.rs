use tree_sitter::{Node, Parser};

use super::coalesce::coalesce;
use super::{Chunk, Span, line_window, lines_chunk};

/// Definitions longer than this are not emitted whole; the walk descends
/// into them so nested definitions become their own chunks.
const MAX_DEF_LINES: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Bash,
    C,
    Cpp,
    CSharp,
    Go,
    Java,
    Javascript,
    Kotlin,
    Lua,
    Php,
    Python,
    Ruby,
    Rust,
    Scala,
    Typescript,
    Tsx,
}

impl Language {
    pub fn from_path(path: &str) -> Option<Self> {
        let ext = path.rsplit_once('.').map(|(_, ext)| ext)?;
        match ext {
            "sh" | "bash" => Some(Self::Bash),
            "c" | "h" => Some(Self::C),
            "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some(Self::Cpp),
            "cs" => Some(Self::CSharp),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::Javascript),
            "kt" | "kts" => Some(Self::Kotlin),
            "lua" => Some(Self::Lua),
            "php" => Some(Self::Php),
            "py" | "pyi" => Some(Self::Python),
            "rb" => Some(Self::Ruby),
            "rs" => Some(Self::Rust),
            "scala" | "sc" => Some(Self::Scala),
            "ts" | "mts" | "cts" => Some(Self::Typescript),
            "tsx" => Some(Self::Tsx),
            _ => None,
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::Bash => tree_sitter_bash::LANGUAGE.into(),
            Self::C => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::Javascript => tree_sitter_javascript::LANGUAGE.into(),
            Self::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Self::Lua => tree_sitter_lua::LANGUAGE.into(),
            Self::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Scala => tree_sitter_scala::LANGUAGE.into(),
            Self::Typescript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }

    fn is_def(self, kind: &str) -> bool {
        match self {
            Self::Bash => kind == "function_definition",
            Self::C => matches!(
                kind,
                "function_definition"
                    | "type_definition"
                    | "struct_specifier"
                    | "enum_specifier"
                    | "union_specifier"
                    | "preproc_function_def"
            ),
            Self::Cpp => matches!(
                kind,
                "function_definition"
                    | "type_definition"
                    | "struct_specifier"
                    | "class_specifier"
                    | "enum_specifier"
                    | "union_specifier"
                    | "namespace_definition"
                    | "template_declaration"
                    | "preproc_function_def"
            ),
            Self::CSharp => matches!(
                kind,
                "class_declaration"
                    | "interface_declaration"
                    | "struct_declaration"
                    | "enum_declaration"
                    | "record_declaration"
                    | "method_declaration"
                    | "constructor_declaration"
                    | "namespace_declaration"
            ),
            Self::Go => matches!(
                kind,
                "function_declaration" | "method_declaration" | "type_declaration"
            ),
            Self::Java => matches!(
                kind,
                "class_declaration"
                    | "interface_declaration"
                    | "enum_declaration"
                    | "record_declaration"
                    | "annotation_type_declaration"
                    | "method_declaration"
                    | "constructor_declaration"
            ),
            Self::Javascript | Self::Typescript | Self::Tsx => matches!(
                kind,
                "function_declaration"
                    | "generator_function_declaration"
                    | "class_declaration"
                    | "abstract_class_declaration"
                    | "method_definition"
                    | "interface_declaration"
                    | "type_alias_declaration"
                    | "enum_declaration"
            ),
            Self::Kotlin => matches!(
                kind,
                "function_declaration"
                    | "class_declaration"
                    | "object_declaration"
                    | "secondary_constructor"
            ),
            Self::Lua => matches!(
                kind,
                "function_declaration" | "function_definition_statement"
            ),
            Self::Php => matches!(
                kind,
                "function_definition"
                    | "method_declaration"
                    | "class_declaration"
                    | "interface_declaration"
                    | "trait_declaration"
                    | "enum_declaration"
            ),
            Self::Python => matches!(
                kind,
                "function_definition" | "class_definition" | "decorated_definition"
            ),
            Self::Ruby => matches!(kind, "method" | "singleton_method" | "class" | "module"),
            Self::Rust => matches!(
                kind,
                "function_item"
                    | "struct_item"
                    | "enum_item"
                    | "union_item"
                    | "trait_item"
                    | "impl_item"
                    | "mod_item"
                    | "macro_definition"
            ),
            Self::Scala => matches!(
                kind,
                "function_definition"
                    | "class_definition"
                    | "object_definition"
                    | "trait_definition"
                    | "enum_definition"
            ),
        }
    }
}

/// C-family definitions bury the identifier in a declarator chain; other
/// grammars without a `name` field still put an identifier-like node among
/// the first named children.
fn fallback_name(node: Node<'_>) -> Option<Node<'_>> {
    let mut current = node;
    while let Some(declarator) = current.child_by_field_name("declarator") {
        current = declarator;
        if current.kind().ends_with("identifier") {
            return Some(current);
        }
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| matches!(child.kind(), k if k.ends_with("identifier") || k == "word" || k == "constant"))
}

fn def_name(language: Language, node: Node<'_>, source: &str) -> Option<String> {
    let named = match (language, node.kind()) {
        (Language::Rust, "impl_item") => node.child_by_field_name("type"),
        (Language::Python, "decorated_definition") => node
            .child_by_field_name("definition")
            .and_then(|def| def.child_by_field_name("name")),
        _ => node
            .child_by_field_name("name")
            .or_else(|| fallback_name(node)),
    }?;
    named.utf8_text(source.as_bytes()).ok().map(str::to_string)
}

/// Comments, attributes, and decorators sitting directly above a
/// definition belong to it; the definition's span grows upward over them.
fn is_trivia(kind: &str) -> bool {
    kind.contains("comment") || kind.contains("attribute") || kind.contains("decorator")
}

struct Walker<'a> {
    language: Language,
    source: &'a str,
    defs: Vec<Span>,
}

impl Walker<'_> {
    fn walk(&mut self, node: Node<'_>, path: &[String]) {
        let mut cursor = node.walk();
        let mut trivia_start: Option<usize> = None;
        let mut last_end_row = 0;
        for child in node.named_children(&mut cursor) {
            let start = child.start_position().row;
            let end = child.end_position().row + 1;
            let adjacent = trivia_start.is_some() && last_end_row >= start;
            last_end_row = end;
            if is_trivia(child.kind()) {
                if !adjacent {
                    trivia_start = Some(start);
                }
                continue;
            }
            let span_start = if adjacent { trivia_start.take() } else { None };
            trivia_start = None;
            if !self.language.is_def(child.kind()) {
                self.walk(child, path);
                continue;
            }
            let name = def_name(self.language, child, self.source);
            if end - start <= MAX_DEF_LINES {
                let mut symbol = path.to_vec();
                symbol.extend(name);
                self.defs.push(Span {
                    start: span_start.unwrap_or(start),
                    end,
                    symbol: (!symbol.is_empty()).then(|| symbol.join(" > ")),
                });
            } else {
                let mut inner = path.to_vec();
                inner.extend(name);
                self.walk(child, &inner);
            }
        }
    }
}

pub fn chunk(language: Language, content: &str) -> Option<Vec<Chunk>> {
    let mut parser = Parser::new();
    parser.set_language(&language.grammar()).ok()?;
    let tree = parser.parse(content, None)?;

    let mut walker = Walker {
        language,
        source: content,
        defs: Vec::new(),
    };
    walker.walk(tree.root_node(), &[]);
    let mut defs = walker.defs;
    defs.sort_by_key(|def| def.start);

    let lines: Vec<&str> = content.lines().collect();
    let mut spans = Vec::new();
    let mut covered = 0;
    for def in defs {
        let start = def.start.max(covered);
        let end = def.end.min(lines.len());
        if start >= end {
            continue;
        }
        spans.extend(line_window::spans(&lines, covered, start));
        spans.push(Span {
            start,
            end,
            symbol: def.symbol,
        });
        covered = end;
    }
    spans.extend(line_window::spans(&lines, covered, lines.len()));
    Some(
        coalesce(spans)
            .into_iter()
            .map(|span| lines_chunk(&lines, span))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbols(chunks: &[Chunk]) -> Vec<&str> {
        chunks.iter().filter_map(|c| c.symbol.as_deref()).collect()
    }

    fn by_symbol<'a>(chunks: &'a [Chunk], symbol: &str) -> &'a Chunk {
        chunks
            .iter()
            .find(|c| c.symbol.as_deref() == Some(symbol))
            .unwrap()
    }

    #[test]
    fn rust_functions_become_symbol_chunks() {
        let source = "use std::io;\n\nfn alpha() {\n    let a = 1;\n    let b = 2;\n    println!(\"{a}{b}\");\n}\n\nstruct Beta {\n    field: u32,\n    other: u32,\n    third: u32,\n}\n";
        let chunks = chunk(Language::Rust, source).unwrap();
        assert_eq!(symbols(&chunks), ["alpha", "Beta"]);
        let alpha = by_symbol(&chunks, "alpha");
        assert_eq!((alpha.start_line, alpha.end_line), (1, 7));
        assert!(alpha.content.contains("println!"));
        assert_eq!(by_symbol(&chunks, "Beta").start_line, 9);
    }

    #[test]
    fn doc_comments_and_attributes_stay_with_their_definition() {
        let source = "fn filler() {\n    1;\n    2;\n    3;\n}\n\n/// Documents alpha.\n/// Second line.\n#[cfg(feature = \"x\")]\nfn alpha() {\n    let a = 1;\n    let b = 2;\n    a + b;\n}\n";
        let chunks = chunk(Language::Rust, source).unwrap();
        let alpha = by_symbol(&chunks, "alpha");
        assert_eq!((alpha.start_line, alpha.end_line), (7, 14));
        assert!(alpha.content.starts_with("/// Documents alpha."));
        assert_eq!(by_symbol(&chunks, "filler").end_line, 5);
    }

    #[test]
    fn detached_comments_are_not_pulled_in() {
        let source =
            "// unrelated note\n\nfn alpha() {\n    let a = 1;\n    let b = 2;\n    a + b;\n}\n";
        let chunks = chunk(Language::Rust, source).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
    }

    #[test]
    fn one_line_declarations_merge_into_one_chunk() {
        let source = "pub(crate) mod cftc;\n#[cfg(feature = \"crypto\")]\npub(crate) mod coingecko;\npub(crate) mod edgar;\n#[cfg(feature = \"finra\")]\npub(crate) mod finra;\n";
        let chunks = chunk(Language::Rust, source).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (1, 6));
        assert_eq!(chunks[0].symbol, None);
    }

    #[test]
    fn small_impl_is_one_chunk_with_type_symbol() {
        let source = "struct S;\n\nimpl S {\n    fn get(&self) -> u32 {\n        1\n    }\n}\n";
        let chunks = chunk(Language::Rust, source).unwrap();
        let imp = chunks
            .iter()
            .find(|c| c.content.contains("fn get"))
            .unwrap();
        assert_eq!(imp.symbol.as_deref(), Some("S"));
    }

    #[test]
    fn oversized_container_descends_to_methods() {
        let body: String = (0..70)
            .map(|i| format!("    fn m{i}(&self) -> u32 {{\n        {i}\n    }}\n"))
            .collect();
        let source = format!("struct S;\n\nimpl S {{\n{body}}}\n");
        let chunks = chunk(Language::Rust, &source).unwrap();
        assert!(
            chunks
                .iter()
                .all(|c| c.symbol.as_deref().is_some_and(|s| s.starts_with("S")))
        );
        assert!(chunks.iter().any(|c| c.content.contains("fn m0(")));
        assert!(chunks.iter().any(|c| c.content.contains("fn m69(")));
        assert!(chunks.iter().all(|c| c.end_line - c.start_line + 1 >= 4));
    }

    #[test]
    fn python_class_and_decorated_defs() {
        let source = "import os\n\nclass Greeter:\n    def hello(self):\n        return 'hi'\n\n    def bye(self):\n        return 'bye'\n\n@cached\ndef top():\n    a = 1\n    b = 2\n    return a + b\n";
        let chunks = chunk(Language::Python, source).unwrap();
        assert!(symbols(&chunks).contains(&"Greeter"));
        assert!(symbols(&chunks).contains(&"top"));
    }

    #[test]
    fn typescript_interfaces_and_functions() {
        let source = "export interface Config {\n  port: number;\n  host: string;\n  tls: boolean;\n}\n\nexport function load(): Config {\n  const port = 1;\n  const host = 'x';\n  return { port, host, tls: false };\n}\n";
        let chunks = chunk(Language::Typescript, source).unwrap();
        assert!(symbols(&chunks).contains(&"Config"));
        assert!(symbols(&chunks).contains(&"load"));
    }

    #[test]
    fn c_functions_resolve_names_through_declarators() {
        let source = "#include <stdio.h>\n\nint add(int a, int b) {\n    int c = a + b;\n    printf(\"%d\", c);\n    return c;\n}\n";
        let chunks = chunk(Language::C, source).unwrap();
        assert!(symbols(&chunks).contains(&"add"));
    }

    #[test]
    fn java_class_methods_get_paths_when_oversized() {
        let source = "class Point {\n    int x;\n\n    int getX() {\n        return x;\n    }\n}\n";
        let chunks = chunk(Language::Java, source).unwrap();
        assert!(symbols(&chunks).contains(&"Point"));
    }

    #[test]
    fn kotlin_functions_and_classes() {
        let source = "class Greeter {\n    fun hello(): String {\n        return \"hi\"\n    }\n}\n\nfun top(): Int {\n    val a = 1\n    val b = 2\n    return a + b\n}\n";
        let chunks = chunk(Language::Kotlin, source).unwrap();
        assert!(symbols(&chunks).contains(&"Greeter"));
        assert!(symbols(&chunks).contains(&"top"));
    }

    #[test]
    fn ruby_classes_and_methods() {
        let source = "class Greeter\n  def hello\n    'hi'\n  end\nend\n";
        let chunks = chunk(Language::Ruby, source).unwrap();
        assert!(symbols(&chunks).contains(&"Greeter"));
    }

    #[test]
    fn bash_functions() {
        let source = "#!/bin/bash\n\ndeploy() {\n  echo deploying\n  echo again\n  echo done\n}\n";
        let chunks = chunk(Language::Bash, source).unwrap();
        assert!(symbols(&chunks).contains(&"deploy"));
    }

    #[test]
    fn gap_lines_still_covered() {
        let source = "const A: u32 = 1;\nconst B: u32 = 2;\n\nfn used() -> u32 {\n    let a = A;\n    let b = B;\n    a + b\n}\n";
        let chunks = chunk(Language::Rust, source).unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("const A"));
        assert_eq!(chunks[0].symbol.as_deref(), Some("used"));
    }
}
