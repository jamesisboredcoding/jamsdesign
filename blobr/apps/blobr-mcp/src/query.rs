//! Finding nodes with CSS-like selectors, for the `query` tool:
//!
//! - `frame`, `text`, `*`: by type (`page`, `frame`, `group`, `section`, `rectangle`,
//!   `ellipse`, `line`, `polygon`, `star`, `vector`, `text`, `component`, `component_set`,
//!   `instance`)
//! - `#12`: by id
//! - `[name=Title]`, `[name*=art]`, `[name^=Btn]`, `[name$=Icon]`: attributes `name`, `text`
//!   (a text layer's characters), `visible`, `locked`, `component` (an instance's component
//!   name); quotes are optional (`[name="Get started"]`)
//! - `frame text` (descendant), `frame > text` (child), `a, b` (either)

use jd_vector::{Document, NodeId, NodeKind};

#[derive(Debug, Clone, PartialEq)]
enum Cmp {
    Has,
    Eq,
    Contains,
    Starts,
    Ends,
}

#[derive(Debug, Clone, PartialEq)]
struct Attr {
    name: String,
    cmp: Cmp,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Compound {
    kind: Option<String>,
    id: Option<u64>,
    attrs: Vec<Attr>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Comb {
    Descendant,
    Child,
}

/// One comma-separated part: compounds joined by combinators (`combs[i]` joins `parts[i]`
/// and `parts[i + 1]`).
#[derive(Debug, Clone, PartialEq, Default)]
struct Chain {
    parts: Vec<Compound>,
    combs: Vec<Comb>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selector(Vec<Chain>);

const TYPES: [&str; 14] =
    ["page", "frame", "group", "section", "rectangle", "ellipse", "line", "polygon", "star", "vector", "text", "component", "component_set", "instance"];
const ATTRS: [&str; 5] = ["name", "text", "visible", "locked", "component"];

impl Selector {
    pub fn parse(s: &str) -> Result<Self, String> {
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        let mut chains = Vec::new();
        let mut chain = Chain::default();
        let mut cur: Option<Compound> = None;
        let mut pending: Option<Comb> = None;
        let err = |m: &str| Err(format!("selector {s:?}: {m}"));
        // Starts a compound if none is open, joining it to the chain with the pending combinator.
        let open = |cur: &mut Option<Compound>, chain: &mut Chain, pending: &mut Option<Comb>| {
            if cur.is_none() {
                if !chain.parts.is_empty() {
                    chain.combs.push(pending.take().unwrap_or(Comb::Descendant));
                }
                *cur = Some(Compound::default());
            }
        };
        while i < chars.len() {
            let c = chars[i];
            match c {
                ' ' | '\t' | '\n' => {
                    if let Some(done) = cur.take() {
                        chain.parts.push(done);
                    }
                    i += 1;
                }
                '>' => {
                    if let Some(done) = cur.take() {
                        chain.parts.push(done);
                    }
                    if chain.parts.is_empty() {
                        return err("'>' needs something before it");
                    }
                    pending = Some(Comb::Child);
                    i += 1;
                }
                ',' => {
                    if let Some(done) = cur.take() {
                        chain.parts.push(done);
                    }
                    if chain.parts.is_empty() || pending.is_some() {
                        return err("empty part before ','");
                    }
                    chains.push(std::mem::take(&mut chain));
                    i += 1;
                }
                '#' => {
                    open(&mut cur, &mut chain, &mut pending);
                    let start = i + 1;
                    i = start;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                    let n: String = chars[start..i].iter().collect();
                    cur.as_mut().expect("open").id = Some(n.parse().map_err(|_| format!("selector {s:?}: '#' needs a node id number"))?);
                }
                '[' => {
                    open(&mut cur, &mut chain, &mut pending);
                    let Some(end) = chars[i..].iter().position(|&c| c == ']') else { return err("unclosed '['") };
                    let body: String = chars[i + 1..i + end].iter().collect();
                    i += end + 1;
                    let (name, cmp, value) = if let Some(p) = body.find(['=', '*', '^', '$']) {
                        let (n, rest) = body.split_at(p);
                        let (cmp, v) = match rest.chars().next() {
                            Some('=') => (Cmp::Eq, &rest[1..]),
                            Some(op) if rest[1..].starts_with('=') => (
                                match op {
                                    '*' => Cmp::Contains,
                                    '^' => Cmp::Starts,
                                    _ => Cmp::Ends,
                                },
                                &rest[2..],
                            ),
                            _ => return err(&format!("bad attribute [{body}]")),
                        };
                        (n.trim().to_owned(), cmp, v.trim().trim_matches(['"', '\'']).to_owned())
                    } else {
                        (body.trim().to_owned(), Cmp::Has, String::new())
                    };
                    if !ATTRS.contains(&name.as_str()) {
                        return err(&format!("unknown attribute {name:?} (use one of {})", ATTRS.join(", ")));
                    }
                    cur.as_mut().expect("open").attrs.push(Attr { name, cmp, value });
                }
                _ if c == '*' || c.is_alphanumeric() || c == '_' => {
                    open(&mut cur, &mut chain, &mut pending);
                    let start = i;
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '*') {
                        i += 1;
                    }
                    let word: String = chars[start..i].iter().collect::<String>().to_lowercase();
                    let comp = cur.as_mut().expect("open");
                    if comp.kind.is_some() || comp.id.is_some() || !comp.attrs.is_empty() {
                        return err(&format!("put the type {word:?} first in each part"));
                    }
                    if word != "*" {
                        if !TYPES.contains(&word.as_str()) {
                            return err(&format!("unknown type {word:?} (use one of {})", TYPES.join(", ")));
                        }
                        comp.kind = Some(word);
                    }
                }
                _ => return err(&format!("unexpected {c:?}")),
            }
        }
        if let Some(done) = cur.take() {
            chain.parts.push(done);
        }
        if pending.is_some() {
            return err("'>' needs something after it");
        }
        if chain.parts.is_empty() {
            return err("empty selector");
        }
        chains.push(chain);
        Ok(Self(chains))
    }

    /// Matching nodes under `root` (not `root` itself), in tree order.
    pub fn find(&self, doc: &Document, root: NodeId) -> Vec<NodeId> {
        doc.descendants(root).into_iter().filter(|&id| self.0.iter().any(|c| chain_matches(doc, c, id, root))).collect()
    }
}

fn compound_matches(doc: &Document, c: &Compound, id: NodeId) -> bool {
    let Some(n) = doc.node(id) else { return false };
    if c.kind.as_deref().is_some_and(|k| k != n.kind.type_name()) || c.id.is_some_and(|i| i != id.0) {
        return false;
    }
    c.attrs.iter().all(|a| {
        let value: Option<String> = match a.name.as_str() {
            "name" => Some(n.name.clone()),
            "text" => match &n.kind {
                NodeKind::Text { text } => Some(text.content.clone()),
                _ => None,
            },
            "visible" => Some(n.props.visible.to_string()),
            "locked" => Some(n.props.locked.to_string()),
            "component" => match &n.kind {
                NodeKind::Instance { component, .. } => doc.node(*component).map(|c| c.name.clone()),
                _ => None,
            },
            _ => None,
        };
        let Some(v) = value else { return false };
        match a.cmp {
            Cmp::Has => true,
            Cmp::Eq => v == a.value,
            Cmp::Contains => v.contains(&a.value),
            Cmp::Starts => v.starts_with(&a.value),
            Cmp::Ends => v.ends_with(&a.value),
        }
    })
}

/// Right to left: the last compound must match `id`, earlier ones its ancestors (up to and
/// including `root`).
fn chain_matches(doc: &Document, chain: &Chain, id: NodeId, root: NodeId) -> bool {
    fn rec(doc: &Document, chain: &Chain, part: usize, id: NodeId, root: NodeId) -> bool {
        if !compound_matches(doc, &chain.parts[part], id) {
            return false;
        }
        if part == 0 {
            return true;
        }
        let mut up = if id == root { None } else { doc.parent(id) };
        match chain.combs[part - 1] {
            Comb::Child => up.is_some_and(|p| rec(doc, chain, part - 1, p, root)),
            Comb::Descendant => {
                while let Some(p) = up {
                    if rec(doc, chain, part - 1, p, root) {
                        return true;
                    }
                    up = if p == root { None } else { doc.parent(p) };
                }
                false
            }
        }
    }
    rec(doc, chain, chain.parts.len() - 1, id, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jd_vector::{ApproxMeasure, NodeSpec, Op, TextStyle};

    fn doc() -> (Document, NodeId) {
        let mut d = Document::new();
        let page = d.pages()[0];
        d.apply(
            &[
                Op::create_as("card", page, NodeSpec::frame("Card", 200.0, 100.0)),
                Op::create("card", NodeSpec::text("Title", "Hello there", TextStyle::default())),
                Op::create_as("row", "card", NodeSpec::frame("Button row", 100.0, 20.0)),
                Op::create("row", NodeSpec::text("Label", "Get started", TextStyle::default())),
                Op::create(page, NodeSpec::rect("Bg", 10.0, 10.0)),
            ],
            &ApproxMeasure,
        )
        .unwrap();
        (d, page)
    }

    fn names(d: &Document, ids: Vec<NodeId>) -> Vec<String> {
        ids.into_iter().map(|i| d.node(i).unwrap().name.clone()).collect()
    }

    #[test]
    fn selectors() {
        let (d, page) = doc();
        let q = |s: &str| names(&d, Selector::parse(s).unwrap().find(&d, page));
        assert_eq!(q("text"), ["Title", "Label"]);
        assert_eq!(q("frame > text"), ["Title", "Label"]);
        assert_eq!(q("frame[name=Card] > text"), ["Title"]);
        assert_eq!(q("frame[name=Card] text"), ["Title", "Label"]);
        assert_eq!(q("[name^=Bu]"), ["Button row"]);
        assert_eq!(q(r#"text[text="Get started"]"#), ["Label"]);
        assert_eq!(q("rectangle, text[text*=there]"), ["Title", "Bg"]);
        assert_eq!(q("page > *").len(), 2);
        let card = d.find_by_name("Card").unwrap();
        assert_eq!(q(&format!("#{}", card.0)), ["Card"]);
        assert!(Selector::parse("button").unwrap_err().contains("unknown type"));
        assert!(Selector::parse("frame >").is_err());
        assert!(Selector::parse("[size=3]").is_err());
    }
}
