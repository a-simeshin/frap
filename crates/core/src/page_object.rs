//! Deterministic Page Object generation from [`ElementMap`].

use crate::element_map::{Cluster, ClusterType, ElementMap, ElementNode};
use crate::CoreError;
use serde::{Deserialize, Serialize};
use std::fmt::Write;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateOptions {
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_class_name")]
    pub class_name: String,
    #[serde(default)]
    pub package_name: Option<String>,
    #[serde(default = "default_true")]
    pub include_signatures: bool,
}

fn default_language() -> String {
    "java_playwright".to_string()
}

fn default_class_name() -> String {
    "GeneratedPage".to_string()
}

fn default_true() -> bool {
    true
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            language: default_language(),
            class_name: default_class_name(),
            package_name: None,
            include_signatures: default_true(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedArtifact {
    pub files: Vec<GeneratedFile>,
}

pub fn generate_page_object(map: &ElementMap, options: &GenerateOptions) -> GeneratedArtifact {
    let content = match options.language.as_str() {
        "java_playwright" => generate_java_playwright(map, options),
        other => format!("// Unsupported language: {other}\n"),
    };

    let path = options
        .package_name
        .as_ref()
        .map(|pkg| format!("{}/{}.java", pkg.replace('.', "/"), options.class_name))
        .unwrap_or_else(|| format!("{}.java", options.class_name));

    GeneratedArtifact {
        files: vec![GeneratedFile { path, content }],
    }
}

pub fn generate_page_object_json(map_json: &str, options_json: &str) -> Result<String, CoreError> {
    let map: ElementMap = serde_json::from_str(map_json)?;
    let options: GenerateOptions = if options_json.trim().is_empty() {
        GenerateOptions::default()
    } else {
        serde_json::from_str(options_json)?
    };
    let artifact = generate_page_object(&map, &options);
    Ok(serde_json::to_string(&artifact)?)
}

fn generate_java_playwright(map: &ElementMap, options: &GenerateOptions) -> String {
    // Render the class body first so we know whether any method used `getByRole`
    // (and therefore needs the `AriaRole` import) before emitting the header.
    let mut body = String::new();

    let mut emitted_clusters = std::collections::HashSet::new();
    // Shared registry of method names already emitted, so cluster and element methods
    // never collide; on collision we append a numeric suffix (2, 3, ...).
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Set to true as soon as a single `getByRole(AriaRole.*, …)` is emitted, gating the
    // conditional `AriaRole` import in the header.
    let mut uses_aria_role = false;

    for cluster in &map.clusters {
        if cluster.cluster_type != ClusterType::List || cluster.element_ids.len() < 2 {
            continue;
        }
        if !emitted_clusters.insert(cluster.id.clone()) {
            continue;
        }
        let sample = cluster
            .element_ids
            .first()
            .and_then(|id| map.elements.iter().find(|e| &e.id == id));
        let method = dedup_name(cluster_method_name(cluster, sample), &mut used_names);
        if let Some(el) = sample {
            writeln!(
                body,
                "\n    /** Cluster `{}` ({} elements) */",
                cluster.id,
                cluster.element_ids.len()
            )
            .unwrap();
            match relative_cluster_emission(cluster) {
                // RELATIVE-локатор (combination 2): the cluster carries a usable container_role +
                // child_role + text-discriminator, so each member is reachable by its visible text
                // instead of a brittle `.nth(index)`.
                Some(rel) => {
                    uses_aria_role = true;
                    writeln!(body, "    public Locator {}(String text) {{", method).unwrap();
                    writeln!(
                        body,
                        "        return page.getByRole(AriaRole.{})",
                        rel.container
                    )
                    .unwrap();
                    writeln!(
                        body,
                        "                   .filter(new Locator.FilterOptions().setHasText(text))"
                    )
                    .unwrap();
                    writeln!(
                        body,
                        "                   .getByRole(AriaRole.{});",
                        rel.child
                    )
                    .unwrap();
                    writeln!(body, "    }}").unwrap();
                }
                // Fallback: no derivable container/child role or no text discriminator — keep the
                // positional `.nth(index)` over the sample's raw STRUCTURAL selector (not its
                // `recommended_selector`, which may be a unique semantic locator valid for a single
                // member but wrong for `.nth(index)`).
                None => {
                    writeln!(body, "    public Locator {}(int index) {{", method).unwrap();
                    writeln!(
                        body,
                        "        return page.locator(\"{}\").nth(index);",
                        escape_java_string(&el.selector)
                    )
                    .unwrap();
                    writeln!(body, "    }}").unwrap();
                }
            }
        }
    }

    for element in &map.elements {
        let method = dedup_name(element_method_name(element), &mut used_names);
        writeln!(
            body,
            "\n    /** {} (confidence {:.2}) */",
            element.tag, element.confidence
        )
        .unwrap();
        if options.include_signatures {
            writeln!(
                body,
                "    // signature prefix: {}",
                element.signature.prefix
            )
            .unwrap();
        }
        // A non-unique recommendation (`match_count > 1`) gets `.first()` plus a doc comment so the
        // user knows the locator is ambiguous and can tighten it.
        if let Some(n) = element.locator.match_count.filter(|&n| n > 1) {
            writeln!(body, "    /** matches {n} — уточните */").unwrap();
        }
        writeln!(body, "    public Locator {}() {{", method).unwrap();
        let LocatorEmission { expr, aria_role } = locator_expression(element);
        uses_aria_role |= aria_role;
        writeln!(body, "        return {};", expr).unwrap();
        writeln!(body, "    }}").unwrap();
    }

    let mut out = String::new();

    if let Some(pkg) = &options.package_name {
        writeln!(out, "package {pkg};\n").unwrap();
    }

    writeln!(out, "import com.microsoft.playwright.Locator;").unwrap();
    writeln!(out, "import com.microsoft.playwright.Page;").unwrap();
    // `AriaRole` is only referenced when at least one method emitted `getByRole`.
    if uses_aria_role {
        writeln!(out, "import com.microsoft.playwright.options.AriaRole;").unwrap();
    }
    out.push('\n');

    writeln!(out, "/** Generated by Frap — do not edit by hand. */").unwrap();
    writeln!(out, "public class {} {{", options.class_name).unwrap();
    writeln!(out, "    private final Page page;\n").unwrap();
    writeln!(out, "    public {}(Page page) {{", options.class_name).unwrap();
    writeln!(out, "        this.page = page;").unwrap();
    writeln!(out, "    }}").unwrap();

    out.push_str(&body);

    writeln!(out, "}}\n").unwrap();
    out
}

/// A rendered Playwright locator expression plus whether it referenced `AriaRole`
/// (so the header can conditionally import it).
struct LocatorEmission {
    expr: String,
    aria_role: bool,
}

/// Map an element's `{strategy, value}` recommendation onto an idiomatic Playwright
/// locator expression. The strategy chooses the `getBy*`/`locator` *call*; an optional
/// `scope` narrows the receiver and an optional `match_count > 1` appends `.first()`:
///
/// | strategy    | call (on `<receiver>`) |
/// |-------------|----------|
/// | `text`      | `getByText("<value>", new Page.GetByTextOptions().setExact(true))` |
/// | `role` + aria-label | `getByLabel("<value>")` |
/// | `role` (no aria-label) | `getByRole(AriaRole.<UPPER>, new Page.GetByRoleOptions().setName("<value>"))` |
/// | `placeholder` | `getByPlaceholder("<value>")` |
/// | `testid` (canonical `[data-testid=…]`) | `getByTestId("<value>")` |
/// | `href` / `id` / `css` / testid alias | `locator("<recommended_selector>")` |
///
/// Receiver is `page` normally, or `page.locator("<scope>")` when `scope` is set (SCOPED locator:
/// the CSS-selector scope is escaped with `escape_java_string` only — whitespace is significant —
/// while the textual `value` is whitespace-normalized + escaped as usual). When `match_count > 1`
/// a `.first()` is appended (the caller also emits a `/** matches N */` comment).
///
/// Any case lacking a usable `value` (or a role with no matching `AriaRole` constant)
/// falls back to `<receiver>.locator("<recommended_selector>")` — never an invalid expression.
fn locator_expression(element: &ElementNode) -> LocatorEmission {
    // SCOPED receiver: `page.locator("<scope>")` when a scope is set, else plain `page`.
    // The scope is a CSS selector — escape only, never whitespace-normalize.
    let receiver = match element.locator.scope.as_deref() {
        Some(scope) => format!("page.locator(\"{}\")", escape_java_string(scope)),
        None => "page".to_string(),
    };

    let fallback = || LocatorEmission {
        expr: format!(
            "{receiver}.locator(\"{}\")",
            escape_java_string(&element.recommended_selector)
        ),
        aria_role: false,
    };

    let value = match element.locator.value.as_deref() {
        Some(v) => v,
        None => return apply_first(fallback(), element),
    };
    // `getBy*` TEXTUAL arguments are matched by Playwright against normalized whitespace, so we
    // collapse whitespace runs (incl. `\n`/`\t`) to a single space + trim *before* escaping.
    // The test-id token (`getByTestId`) is an identifier, not free text — only escape it.
    let escaped_text = escape_java_string(&normalize_whitespace(value));
    let escaped_raw = escape_java_string(value);

    let emission = match element.locator.strategy.as_str() {
        "text" => LocatorEmission {
            expr: format!(
                "{receiver}.getByText(\"{escaped_text}\", new Page.GetByTextOptions().setExact(true))"
            ),
            aria_role: false,
        },
        "role" => {
            if element.signature.stable_attrs.contains_key("aria-label") {
                LocatorEmission {
                    expr: format!("{receiver}.getByLabel(\"{escaped_text}\")"),
                    aria_role: false,
                }
            } else {
                match aria_role_constant(element) {
                    Some(role) => LocatorEmission {
                        expr: format!(
                            "{receiver}.getByRole(AriaRole.{role}, new Page.GetByRoleOptions().setName(\"{escaped_text}\"))"
                        ),
                        aria_role: true,
                    },
                    None => fallback(),
                }
            }
        }
        "placeholder" => LocatorEmission {
            expr: format!("{receiver}.getByPlaceholder(\"{escaped_text}\")"),
            aria_role: false,
        },
        "testid" if element.recommended_selector.starts_with("[data-testid=") => LocatorEmission {
            expr: format!("{receiver}.getByTestId(\"{escaped_raw}\")"),
            aria_role: false,
        },
        // `href` / `id` / `css` / non-canonical testid aliases stay as CSS locators.
        _ => fallback(),
    };

    apply_first(emission, element)
}

/// Append `.first()` to a locator expression when the recommendation matches more than one element
/// snapshot-wide (`match_count > 1`). A unique locator (`None` / `Some(1)`) is returned unchanged.
fn apply_first(mut emission: LocatorEmission, element: &ElementNode) -> LocatorEmission {
    if matches!(element.locator.match_count, Some(n) if n > 1) {
        emission.expr.push_str(".first()");
    }
    emission
}

/// Container + child `AriaRole.<UPPER>` constants for a RELATIVE List-cluster locator.
struct RelativeClusterEmission {
    container: &'static str,
    child: &'static str,
}

/// Decide whether a List cluster can emit a RELATIVE locator (combination 2) instead of the
/// positional `.nth(index)` fallback.
///
/// Requirements (all must hold):
/// - `variable_kind == Some("text")` — members are distinguished by visible text, so `setHasText`
///   is a valid discriminator (an `href`/`None` variable kind keeps the `.nth(index)` fallback in
///   this iteration);
/// - `container_role` maps to a known `AriaRole` constant;
/// - `child_role` maps to a known `AriaRole` constant.
///
/// Returns `None` (caller emits `.nth(index)`) if any requirement is unmet.
fn relative_cluster_emission(cluster: &Cluster) -> Option<RelativeClusterEmission> {
    if cluster.variable_kind.as_deref() != Some("text") {
        return None;
    }
    let container = cluster_role_constant(cluster.container_role.as_deref())?;
    let child = cluster_role_constant(cluster.child_role.as_deref())?;
    Some(RelativeClusterEmission { container, child })
}

/// Map a cluster `container_role`/`child_role` string onto its `AriaRole.<NAME>` constant.
///
/// The role may be either a raw ARIA role (`"listitem"`, `"link"`) or a `scope_hint`/tag-derived
/// value; both `aria_role_name` and a small tag-fallback are tried so common containers
/// (`list`/`listitem`/`navigation`) and child controls (`link`/`button`) resolve. Returns `None`
/// for any role with no matching `AriaRole` constant, so the caller falls back to `.nth(index)`
/// rather than emit a non-existent constant (which would not compile).
fn cluster_role_constant(role: Option<&str>) -> Option<&'static str> {
    let role = role?.trim();
    if role.is_empty() {
        return None;
    }
    let lower = role.to_ascii_lowercase();
    if let Some(c) = aria_role_name(&lower) {
        return Some(c);
    }
    // `aria_role_name` covers the interactive/landmark roles; add the list family used as a
    // repeated-component container.
    match lower.as_str() {
        "list" => Some("LIST"),
        "listitem" => Some("LISTITEM"),
        "row" => Some("ROW"),
        "cell" | "gridcell" => Some("CELL"),
        "listbox" => Some("LISTBOX"),
        "menu" => Some("MENU"),
        "menubar" => Some("MENUBAR"),
        "table" => Some("TABLE"),
        "grid" => Some("GRID"),
        "article" => Some("ARTICLE"),
        "group" => Some("GROUP"),
        _ => None,
    }
}

/// Resolve the `AriaRole.<UPPER>` constant for a `role`-strategy element. The role is taken
/// from `signature.stable_attrs["role"]` when present, otherwise inferred from the tag
/// (`a→LINK`, `button→BUTTON`, `input[type]→TEXTBOX/CHECKBOX/RADIO/BUTTON`, `textarea→TEXTBOX`,
/// `select→COMBOBOX`, `img→IMG`, `nav→NAVIGATION`). Returns `None` when no known `AriaRole`
/// constant exists, so the caller can fall back to `page.locator(selector)` rather than emit
/// a non-existent constant (which would not compile).
fn aria_role_constant(element: &ElementNode) -> Option<&'static str> {
    let attrs = &element.signature.stable_attrs;
    if let Some(role) = attrs
        .get("role")
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
    {
        return aria_role_name(&role.to_ascii_lowercase());
    }
    let tag = element.tag.to_ascii_lowercase();
    match tag.as_str() {
        "a" => Some("LINK"),
        "button" => Some("BUTTON"),
        "select" => Some("COMBOBOX"),
        "textarea" => Some("TEXTBOX"),
        "img" => Some("IMG"),
        "nav" => Some("NAVIGATION"),
        "input" => {
            let input_type = attrs
                .get("type")
                .map(|t| t.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "text".to_string());
            match input_type.as_str() {
                "text" | "search" | "email" | "tel" | "url" | "password" => Some("TEXTBOX"),
                "checkbox" => Some("CHECKBOX"),
                "radio" => Some("RADIO"),
                "button" | "submit" | "reset" => Some("BUTTON"),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Map an ARIA role string (lowercase) to its `AriaRole.<NAME>` enum constant. Covers the
/// roles produced by the cascade (`implicit_role`) plus common explicit roles. Returns `None`
/// for roles with no matching `AriaRole` constant so the caller can fall back to a CSS locator.
fn aria_role_name(role: &str) -> Option<&'static str> {
    match role {
        "link" => Some("LINK"),
        "button" => Some("BUTTON"),
        "textbox" => Some("TEXTBOX"),
        "combobox" => Some("COMBOBOX"),
        "checkbox" => Some("CHECKBOX"),
        "radio" => Some("RADIO"),
        "navigation" => Some("NAVIGATION"),
        "img" => Some("IMG"),
        "tab" => Some("TAB"),
        "tabpanel" => Some("TABPANEL"),
        "menuitem" => Some("MENUITEM"),
        "option" => Some("OPTION"),
        "heading" => Some("HEADING"),
        "switch" => Some("SWITCH"),
        "searchbox" => Some("SEARCHBOX"),
        "dialog" => Some("DIALOG"),
        "alert" => Some("ALERT"),
        "banner" => Some("BANNER"),
        "main" => Some("MAIN"),
        _ => None,
    }
}

/// Maximum number of words kept from a name source before truncation, so generated
/// method names stay short and readable (e.g. "Войти в личный кабинет онлайн" -> 4 words).
const MAX_NAME_WORDS: usize = 4;

/// Derive a readable method name for an element.
///
/// Source priority:
/// 1. visible `text_content`
/// 2. `aria-label` (accessible name)
/// 3. `placeholder`
///
/// The chosen source is transliterated RU->latin, reduced to alphanumeric words,
/// truncated to the first few words and camelCased, then a role suffix is appended
/// (a -> Link, button -> Button, input/textarea -> Input, select -> Select).
///
/// When no textual source exists, falls back to the legacy `tag + strategy + id` scheme.
fn element_method_name(element: &ElementNode) -> String {
    let source = name_source(element);

    match source {
        Some(raw) => {
            let base = humanize(&raw);
            if base.is_empty() {
                return fallback_element_name(element);
            }
            let suffix = tag_suffix(&element.tag);
            // Prefix the role/tag word *before* the base when the base starts with a digit
            // (e.g. phone-number text "880050087" -> `link880050087`), so the resulting
            // identifier never begins with a digit (`^[A-Za-z_]`). The trailing suffix is kept
            // for the normal (non-digit-leading) case.
            let camel = if first_alnum_is_digit(&base) && !suffix.is_empty() {
                to_camel_case(&format!("{suffix}_{base}"))
            } else {
                to_camel_case(&format!("{base}_{suffix}"))
            };
            ensure_valid_identifier(camel, element)
        }
        None => fallback_element_name(element),
    }
}

/// `true` when the first alphanumeric character of `s` is an ASCII digit. Used to detect
/// name bases that would otherwise produce an invalid (digit-leading) Java identifier.
fn first_alnum_is_digit(s: &str) -> bool {
    s.chars()
        .find(|c| c.is_alphanumeric())
        .is_some_and(|c| c.is_ascii_digit())
}

/// Guarantee a valid Java identifier (`^[A-Za-z_][A-Za-z0-9_]*$`). Empty -> legacy fallback;
/// a digit-leading name (no usable role/tag prefix existed) is rescued by prefixing the legacy
/// fallback base so the identifier always starts with a letter.
fn ensure_valid_identifier(name: String, element: &ElementNode) -> String {
    if name.is_empty() {
        return fallback_element_name(element);
    }
    if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return fallback_element_name(element);
    }
    name
}

/// Pick the best human-readable source string for an element name.
fn name_source(element: &ElementNode) -> Option<String> {
    let sig = &element.signature;
    if let Some(text) = sig.text_content.as_ref() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    for key in ["aria-label", "placeholder"] {
        if let Some(value) = sig.stable_attrs.get(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Legacy deterministic name: `tag + strategy + id`, used when no text source exists.
fn fallback_element_name(element: &ElementNode) -> String {
    let base = element
        .tag
        .chars()
        .chain(element.locator.strategy.chars())
        .filter(|c| c.is_alphanumeric())
        .collect::<String>();
    let suffix = element.id.replace('-', "");
    to_camel_case(&format!("{base}_{suffix}"))
}

/// Role/tag suffix appended to semantic element names.
fn tag_suffix(tag: &str) -> &'static str {
    match tag.to_ascii_lowercase().as_str() {
        "a" => "Link",
        "button" => "Button",
        "input" | "textarea" => "Input",
        "select" => "Select",
        _ => "",
    }
}

/// Derive a readable method name for a list cluster.
///
/// Prefers the common interactive tag/role of the cluster's sample element
/// (`<tag>Items`, e.g. `articleItems`), otherwise the sample's visible text plus
/// `List` (e.g. `cardList`). Falls back to the prefix signature when neither
/// yields anything usable.
fn cluster_method_name(cluster: &Cluster, sample: Option<&ElementNode>) -> String {
    let fallback = || {
        let sanitized: String = cluster
            .prefix_signature
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        // `cluster_` prefix keeps the identifier letter-leading even when `sanitized` is
        // empty or starts with a digit.
        to_camel_case(&format!("cluster_{sanitized}"))
    };

    if let Some(el) = sample {
        let tag = el.tag.trim();
        if !tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphanumeric()) {
            return to_camel_case(&format!("{tag}_items"));
        }
        if let Some(raw) = name_source(el) {
            let base = humanize(&raw);
            if !base.is_empty() {
                // A text base starting with a digit would yield an invalid identifier
                // (`880050087List`); prefix the role/tag word in that case (`link880050087List`).
                let suffix = tag_suffix(&el.tag);
                let name = if first_alnum_is_digit(&base) && !suffix.is_empty() {
                    to_camel_case(&format!("{suffix}_{base}_list"))
                } else {
                    to_camel_case(&format!("{base}_list"))
                };
                if !name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    return name;
                }
            }
        }
    }
    fallback()
}

/// Transliterate RU->latin, keep only alphanumerics (as word separators), and
/// truncate to the first [`MAX_NAME_WORDS`] words. Returns a space-joined string
/// ready for [`to_camel_case`]. Empty when the source has no usable characters.
fn humanize(raw: &str) -> String {
    let translit = transliterate(raw);
    let words: Vec<String> = translit
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(MAX_NAME_WORDS)
        .map(|w| w.to_string())
        .collect();
    words.join(" ")
}

/// Deterministic Cyrillic->latin transliteration. Non-Cyrillic characters pass
/// through unchanged.
fn transliterate(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            'а' => out.push('a'),
            'б' => out.push('b'),
            'в' => out.push('v'),
            'г' => out.push('g'),
            'д' => out.push('d'),
            'е' | 'ё' => out.push('e'),
            'ж' => out.push_str("zh"),
            'з' => out.push('z'),
            'и' | 'й' => out.push('i'),
            'к' => out.push('k'),
            'л' => out.push('l'),
            'м' => out.push('m'),
            'н' => out.push('n'),
            'о' => out.push('o'),
            'п' => out.push('p'),
            'р' => out.push('r'),
            'с' => out.push('s'),
            'т' => out.push('t'),
            'у' => out.push('u'),
            'ф' => out.push('f'),
            'х' => out.push('h'),
            'ц' => out.push('c'),
            'ч' => out.push_str("ch"),
            'ш' => out.push_str("sh"),
            'щ' => out.push_str("sch"),
            'ъ' | 'ь' => {}
            'ы' => out.push('y'),
            'э' => out.push('e'),
            'ю' => out.push_str("yu"),
            'я' => out.push_str("ya"),
            'А' => out.push('A'),
            'Б' => out.push('B'),
            'В' => out.push('V'),
            'Г' => out.push('G'),
            'Д' => out.push('D'),
            'Е' | 'Ё' => out.push('E'),
            'Ж' => out.push_str("Zh"),
            'З' => out.push('Z'),
            'И' | 'Й' => out.push('I'),
            'К' => out.push('K'),
            'Л' => out.push('L'),
            'М' => out.push('M'),
            'Н' => out.push('N'),
            'О' => out.push('O'),
            'П' => out.push('P'),
            'Р' => out.push('R'),
            'С' => out.push('S'),
            'Т' => out.push('T'),
            'У' => out.push('U'),
            'Ф' => out.push('F'),
            'Х' => out.push('H'),
            'Ц' => out.push('C'),
            'Ч' => out.push_str("Ch"),
            'Ш' => out.push_str("Sh"),
            'Щ' => out.push_str("Sch"),
            'Ъ' | 'Ь' => {}
            'Ы' => out.push('Y'),
            'Э' => out.push('E'),
            'Ю' => out.push_str("Yu"),
            'Я' => out.push_str("Ya"),
            other => out.push(other),
        }
    }
    out
}

/// Ensure `name` is unique within `used`; on collision append the smallest free
/// numeric suffix (2, 3, ...). The returned name is recorded in `used`.
fn dedup_name(name: String, used: &mut std::collections::HashSet<String>) -> String {
    if used.insert(name.clone()) {
        return name;
    }
    let mut n = 2usize;
    loop {
        let candidate = format!("{name}{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

fn to_camel_case(input: &str) -> String {
    let mut out = String::new();
    let mut upper_next = false;
    for c in input.chars() {
        if !c.is_alphanumeric() {
            upper_next = true;
            continue;
        }
        if out.is_empty() {
            out.push(c.to_ascii_lowercase());
        } else if upper_next {
            out.push(c.to_ascii_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
    }
    if out.is_empty() {
        "element".to_string()
    } else {
        out
    }
}

/// Escape a string into a valid Java string-literal body (without the surrounding quotes).
///
/// Beyond `\\` and `"`, the common control characters carried by `text_content`/`value`
/// (`\n`, `\r`, `\t`) are turned into their Java escape sequences, and any remaining control
/// character (`< 0x20`) is emitted as a `\uXXXX` unicode escape. This guarantees that every
/// emitted literal stays on a single source line and compiles — for `page.locator` selectors
/// and `getBy*` arguments alike.
fn escape_java_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                write!(out, "\\u{:04x}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out
}

/// Collapse every run of whitespace (including `\n`/`\t`/`\r`) into a single space and trim.
///
/// Applied to `getBy*` textual arguments (`getByText` / `getByLabel` / `getByRole` name /
/// `getByPlaceholder`) *before* escaping: Playwright's `getByText` exact match compares against
/// normalized whitespace, so the literal we emit should match what Playwright actually sees.
/// CSS selectors (`page.locator`) are NOT normalized — whitespace is significant there.
fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element_map::{build_element_map, LocatorRecommendation, MapMetadata, MapOptions};
    use crate::DOMSnapshot;
    use std::collections::HashMap;

    #[test]
    fn generates_java_with_class_and_locators() {
        let snapshot = DOMSnapshot {
            html: String::new(),
            elements: vec![crate::DOMElementInfo {
                selector: "[data-testid='submit']".to_string(),
                tag: "button".to_string(),
                attributes: [("data-testid".to_string(), "submit".to_string())].into(),
                text_content: Some("OK".to_string()),
                path: vec!["button:-".to_string()],
                position_in_parent: None,
                visible: None,
                computed_role: None,
                accessible_name: None,
                scope_hint: None,
            }],
        };
        let map = build_element_map(&snapshot, &MapOptions::default());
        let artifact = generate_page_object(
            &map,
            &GenerateOptions {
                class_name: "CheckoutPage".to_string(),
                package_name: Some("com.example.pages".to_string()),
                ..Default::default()
            },
        );
        let source = &artifact.files[0].content;
        assert!(source.contains("public class CheckoutPage"));
        assert!(source.contains("package com.example.pages"));
        // A canonical `data-testid` element now emits the semantic `getByTestId` locator
        // instead of a positional `page.locator(...)`.
        assert!(
            source.contains("public Locator"),
            "missing generated locator method:\n{source}"
        );
        assert!(
            source.contains("getByTestId(\"submit\")"),
            "expected getByTestId for canonical data-testid:\n{source}"
        );
    }

    #[test]
    fn method_name_from_visible_text_translit() {
        let snapshot = DOMSnapshot {
            html: String::new(),
            elements: vec![crate::DOMElementInfo {
                selector: "a.nav".to_string(),
                tag: "a".to_string(),
                attributes: [("href".to_string(), "/private".to_string())].into(),
                text_content: Some("Частным клиентам".to_string()),
                path: vec!["a:-".to_string()],
                position_in_parent: None,
                visible: None,
                computed_role: None,
                accessible_name: None,
                scope_hint: None,
            }],
        };
        let map = build_element_map(&snapshot, &MapOptions::default());
        let method = element_method_name(&map.elements[0]);
        assert_eq!(method, "chastnymKlientamLink", "got {method}");
    }

    #[test]
    fn method_names_are_deduplicated_on_collision() {
        let snapshot = DOMSnapshot {
            html: String::new(),
            elements: vec![
                crate::DOMElementInfo {
                    selector: "button.a".to_string(),
                    tag: "button".to_string(),
                    attributes: HashMap::new(),
                    text_content: Some("Купить".to_string()),
                    path: vec!["button:-".to_string()],
                    position_in_parent: None,
                    visible: None,
                    computed_role: None,
                    accessible_name: None,
                    scope_hint: None,
                },
                crate::DOMElementInfo {
                    selector: "button.b".to_string(),
                    tag: "button".to_string(),
                    attributes: HashMap::new(),
                    text_content: Some("Купить".to_string()),
                    path: vec!["button:-".to_string()],
                    position_in_parent: None,
                    visible: None,
                    computed_role: None,
                    accessible_name: None,
                    scope_hint: None,
                },
            ],
        };
        let map = build_element_map(&snapshot, &MapOptions::default());
        let artifact = generate_page_object(&map, &GenerateOptions::default());
        let source = &artifact.files[0].content;

        // Both elements share the same text -> base name `kupitButton`. The second
        // must be disambiguated with a numeric suffix.
        assert!(
            source.contains("public Locator kupitButton()"),
            "missing base method:\n{source}"
        );
        assert!(
            source.contains("public Locator kupitButton2()"),
            "missing deduplicated method:\n{source}"
        );
    }

    #[test]
    fn falls_back_to_legacy_name_without_text() {
        let element = ElementNode {
            id: "el-7".to_string(),
            selector: "div.x".to_string(),
            recommended_selector: "div.x".to_string(),
            tag: "div".to_string(),
            signature: signature::Signature {
                path: vec![],
                prefix: String::new(),
                stable_attrs: HashMap::new(),
                text_content: None,
                position_in_parent: None,
                children_hash: 0,
                depth: 0,
            },
            cluster_id: None,
            confidence: 0.4,
            locator: LocatorRecommendation {
                selector: "div.x".to_string(),
                strategy: "css".to_string(),
                confidence: 0.4,
                value: None,
                scope: None,
                filter_text: None,
                match_count: None,
            },
        };
        // No text/aria/placeholder -> legacy tag+strategy+id scheme, never empty.
        assert_eq!(element_method_name(&element), "divcssEl7");
    }

    /// Build an `ElementNode` directly for generator-level tests, bypassing the cascade so we
    /// can pin an exact `{strategy, value}` and `stable_attrs` combination.
    #[allow(clippy::too_many_arguments)]
    fn node(
        id: &str,
        tag: &str,
        selector: &str,
        recommended_selector: &str,
        strategy: &str,
        value: Option<&str>,
        text: Option<&str>,
        stable_attrs: &[(&str, &str)],
    ) -> ElementNode {
        ElementNode {
            id: id.to_string(),
            selector: selector.to_string(),
            recommended_selector: recommended_selector.to_string(),
            tag: tag.to_string(),
            signature: signature::Signature {
                path: vec![],
                prefix: String::new(),
                stable_attrs: stable_attrs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                text_content: text.map(|t| t.to_string()),
                position_in_parent: None,
                children_hash: 0,
                depth: 0,
            },
            cluster_id: None,
            confidence: 0.9,
            locator: LocatorRecommendation {
                selector: recommended_selector.to_string(),
                strategy: strategy.to_string(),
                confidence: 0.9,
                value: value.map(|v| v.to_string()),
                scope: None,
                filter_text: None,
                match_count: None,
            },
        }
    }

    #[test]
    fn method_name_never_starts_with_digit() {
        // Visible text that is purely digits must not yield a digit-leading identifier.
        let snapshot = DOMSnapshot {
            html: String::new(),
            elements: vec![crate::DOMElementInfo {
                selector: "a.phone".to_string(),
                tag: "a".to_string(),
                attributes: [("href".to_string(), "tel:880050087".to_string())].into(),
                text_content: Some("880050087".to_string()),
                path: vec!["a:-".to_string()],
                position_in_parent: None,
                visible: None,
                computed_role: None,
                accessible_name: None,
                scope_hint: None,
            }],
        };
        let map = build_element_map(&snapshot, &MapOptions::default());
        let method = element_method_name(&map.elements[0]);
        assert!(
            method
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_'),
            "method name must start with a letter or underscore, got `{method}`"
        );
        assert!(
            method
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "method name must be a valid Java identifier, got `{method}`"
        );
    }

    #[test]
    fn method_name_sanitizes_phone_number_text() {
        // `<a>880050087</a>`: digit-leading text gets the role/tag word prefixed.
        let element = node(
            "el-1",
            "a",
            "a.phone",
            "[href=\"tel:880050087\"]",
            "href",
            Some("tel:880050087"),
            Some("880050087"),
            &[],
        );
        let method = element_method_name(&element);
        assert_eq!(method, "link880050087", "got `{method}`");
    }

    #[test]
    fn getbytext_emitted_for_text_strategy() {
        let element = node(
            "el-1",
            "button",
            "button.x",
            "button.x",
            "text",
            Some("Войти"),
            Some("Войти"),
            &[],
        );
        let map = single_element_map(element);
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;
        assert!(
            source.contains("getByText("),
            "expected getByText for text strategy:\n{source}"
        );
        assert!(
            source.contains("setExact(true)"),
            "expected exact text match:\n{source}"
        );
    }

    #[test]
    fn getbylabel_emitted_for_role_strategy() {
        // strategy `role` WITH aria-label in stable_attrs -> getByLabel.
        let element = node(
            "el-1",
            "button",
            "button.x",
            "button.x",
            "role",
            Some("Close dialog"),
            None,
            &[("aria-label", "Close dialog")],
        );
        let map = single_element_map(element);
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;
        assert!(
            source.contains("getByLabel(\"Close dialog\")"),
            "expected getByLabel for role+aria-label:\n{source}"
        );
    }

    #[test]
    fn getbyrole_emitted_for_role_strategy_without_aria_label() {
        // strategy `role` WITHOUT aria-label -> getByRole(AriaRole.<UPPER>, name=…) + import.
        let element = node(
            "el-1",
            "a",
            "nav > a",
            "nav > a",
            "role",
            Some("Sign in"),
            Some("Sign in"),
            &[],
        );
        let map = single_element_map(element);
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;
        assert!(
            source.contains("getByRole(AriaRole.LINK"),
            "expected getByRole(AriaRole.LINK) for an anchor role:\n{source}"
        );
        assert!(
            source.contains("setName(\"Sign in\")"),
            "expected accessible name on the role locator:\n{source}"
        );
        assert!(
            source.contains("import com.microsoft.playwright.options.AriaRole;"),
            "AriaRole must be imported when getByRole is emitted:\n{source}"
        );
    }

    #[test]
    fn aria_role_import_absent_without_getbyrole() {
        // A purely text/testid page must not pull in the AriaRole import.
        let element = node(
            "el-1",
            "button",
            "button.x",
            "button.x",
            "text",
            Some("OK"),
            Some("OK"),
            &[],
        );
        let map = single_element_map(element);
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;
        assert!(
            !source.contains("AriaRole"),
            "AriaRole import must be absent when no getByRole is emitted:\n{source}"
        );
    }

    #[test]
    fn getbyplaceholder_emitted_for_placeholder_strategy() {
        let element = node(
            "el-1",
            "input",
            "input.search",
            "[placeholder=\"Search\"]",
            "placeholder",
            Some("Search"),
            None,
            &[("placeholder", "Search")],
        );
        let map = single_element_map(element);
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;
        assert!(
            source.contains("getByPlaceholder(\"Search\")"),
            "expected getByPlaceholder for placeholder strategy:\n{source}"
        );
    }

    #[test]
    fn getbytestid_for_canonical_testid_but_locator_for_alias() {
        // Canonical data-testid -> getByTestId; an alias (data-qa) -> page.locator(selector).
        let canonical = node(
            "el-1",
            "button",
            "[data-testid=\"submit\"]",
            "[data-testid=\"submit\"]",
            "testid",
            Some("submit"),
            None,
            &[],
        );
        let alias = node(
            "el-2",
            "button",
            "[data-qa=\"cancel\"]",
            "[data-qa=\"cancel\"]",
            "testid",
            Some("cancel"),
            None,
            &[],
        );
        let map = ElementMap {
            elements: vec![canonical, alias],
            clusters: vec![],
            metadata: MapMetadata {
                url: None,
                element_count: 2,
                cluster_count: 0,
                timestamp_ms: 0,
            },
        };
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;
        assert!(
            source.contains("getByTestId(\"submit\")"),
            "canonical data-testid -> getByTestId:\n{source}"
        );
        assert!(
            source.contains("page.locator(\"[data-qa=\\\"cancel\\\"]\")"),
            "testid alias -> page.locator(selector):\n{source}"
        );
    }

    #[test]
    fn href_strategy_stays_page_locator() {
        let element = node(
            "el-1",
            "a",
            "a.home",
            "[href=\"/home\"]",
            "href",
            Some("/home"),
            Some("Home"),
            &[],
        );
        let map = single_element_map(element);
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;
        assert!(
            source.contains("page.locator(\"[href=\\\"/home\\\"]\")"),
            "href strategy must stay page.locator:\n{source}"
        );
        assert!(
            !source.contains("getBy"),
            "href strategy must not emit a getBy* locator:\n{source}"
        );
    }

    #[test]
    fn getbytext_value_with_newlines_is_escaped_and_normalized() {
        // text_content/value carrying `\n` + indentation must not produce a multiline (unclosed)
        // Java string literal. The argument is whitespace-normalized then escaped, so it stays on
        // a single line and matches what Playwright's `getByText` exact comparison sees.
        let element = node(
            "el-1",
            "button",
            "button.x",
            "button.x",
            "text",
            Some("Phone\n    Buy"),
            Some("Phone\n    Buy"),
            &[],
        );
        let map = single_element_map(element);
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;

        // No raw newline inside the generated literal (would break compilation).
        assert!(
            !source.contains("Phone\n"),
            "getByText literal must not contain a raw newline:\n{source}"
        );
        // Whitespace collapsed to a single space, on one line, exact-matched.
        assert!(
            source.contains(
                "page.getByText(\"Phone Buy\", new Page.GetByTextOptions().setExact(true))"
            ),
            "expected normalized + escaped getByText argument:\n{source}"
        );
    }

    #[test]
    fn page_locator_selector_escapes_control_chars_without_normalizing() {
        // CSS selectors are NOT whitespace-normalized (whitespace is significant) but control
        // characters still get escaped so the literal compiles.
        let element = node(
            "el-1",
            "div",
            "div.x",
            "div[data-x=\"a\tb\"]",
            "css",
            None,
            None,
            &[],
        );
        let map = single_element_map(element);
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;
        assert!(
            source.contains("page.locator(\"div[data-x=\\\"a\\tb\\\"]\")"),
            "selector tab must be escaped, not normalized away:\n{source}"
        );
    }

    #[test]
    fn generator_emits_relative_filter_locator_for_list() {
        // A List cluster carrying container_role/child_role + a `text` discriminator must emit a
        // RELATIVE locator `getByRole(container).filter(setHasText(text)).getByRole(child)` taking a
        // `String text` parameter, NOT the positional `.nth(index)` fallback.
        let member_a = node(
            "el-0",
            "a",
            "ul > li:nth-child(1) > a",
            "ul > li:nth-child(1) > a",
            "css",
            None,
            Some("First"),
            &[],
        );
        let member_b = node(
            "el-1",
            "a",
            "ul > li:nth-child(2) > a",
            "ul > li:nth-child(2) > a",
            "css",
            None,
            Some("Second"),
            &[],
        );
        let cluster = Cluster {
            id: "cluster-0".to_string(),
            cluster_type: ClusterType::List,
            element_ids: vec!["el-0".to_string(), "el-1".to_string()],
            prefix_signature: "ul>li>a".to_string(),
            container_role: Some("listitem".to_string()),
            child_role: Some("link".to_string()),
            variable_kind: Some("text".to_string()),
        };
        let map = ElementMap {
            elements: vec![member_a, member_b],
            clusters: vec![cluster],
            metadata: MapMetadata {
                url: None,
                element_count: 2,
                cluster_count: 1,
                timestamp_ms: 0,
            },
        };
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;

        // Parametric signature, not `(int index)`.
        assert!(
            source.contains("(String text) {"),
            "relative cluster method must take a String text parameter:\n{source}"
        );
        assert!(
            !source.contains(".nth(index)"),
            "relative cluster must not fall back to .nth(index):\n{source}"
        );
        // Relative chain: container role -> filter(setHasText) -> child role.
        assert!(
            source.contains("page.getByRole(AriaRole.LISTITEM)"),
            "expected container AriaRole.LISTITEM:\n{source}"
        );
        assert!(
            source.contains("new Locator.FilterOptions().setHasText(text)"),
            "expected filter(setHasText(text)):\n{source}"
        );
        assert!(
            source.contains("getByRole(AriaRole.LINK)"),
            "expected child AriaRole.LINK:\n{source}"
        );
        assert!(
            source.contains("import com.microsoft.playwright.options.AriaRole;"),
            "AriaRole must be imported when a relative locator is emitted:\n{source}"
        );
    }

    #[test]
    fn generator_emits_first_for_match_count_above_one() {
        // A recommendation matching more than one element snapshot-wide gets `.first()` plus a
        // `/** matches N */` doc comment.
        let mut element = node(
            "el-1",
            "button",
            "button.buy",
            "button.buy",
            "text",
            Some("Buy"),
            Some("Buy"),
            &[],
        );
        element.locator.match_count = Some(3);
        let map = single_element_map(element);
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;

        assert!(
            source.contains(".first()"),
            "match_count > 1 must append .first():\n{source}"
        );
        assert!(
            source.contains("/** matches 3 — уточните */"),
            "expected the matches-N doc comment:\n{source}"
        );
    }

    #[test]
    fn generator_emits_scoped_locator_when_scope_present() {
        // A `scope` on the recommendation narrows the receiver to `page.locator("<scope>")` before
        // applying the strategy. The CSS scope is escape-only (whitespace significant); the text
        // value is whitespace-normalized + escaped.
        let mut element = node(
            "el-1",
            "button",
            "button.x",
            "button.x",
            "text",
            Some("OK"),
            Some("OK"),
            &[],
        );
        element.locator.scope = Some("nav#main".to_string());
        let map = single_element_map(element);
        let source = &generate_page_object(&map, &GenerateOptions::default()).files[0].content;

        assert!(
            source.contains(
                "page.locator(\"nav#main\").getByText(\"OK\", new Page.GetByTextOptions().setExact(true))"
            ),
            "expected scoped getByText receiver:\n{source}"
        );
    }

    /// Wrap a single node in an `ElementMap` (no clusters) for generator tests.
    fn single_element_map(element: ElementNode) -> ElementMap {
        ElementMap {
            elements: vec![element],
            clusters: vec![],
            metadata: MapMetadata {
                url: None,
                element_count: 1,
                cluster_count: 0,
                timestamp_ms: 0,
            },
        }
    }
}
