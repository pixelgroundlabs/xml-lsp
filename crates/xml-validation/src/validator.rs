//! The tree walker: drives the derivative machine over a document and
//! turns failed derivatives into diagnostics with precise positions.
//!
//! Error recovery keeps validation going: an invalid event is reported
//! once and then ignored (or handled leniently), so one mistake does not
//! cascade through the rest of the document.

use xml_ast::{unescape, Document, Element, NodeId, NodeKind, TextRange, XMLNS_NAMESPACE};
use xml_schema::{NameClass, PatternId, Schema};

use crate::diagnostics::{ValidationDiagnostic, ValidationDiagnosticCode};
use crate::machine::{Machine, VId, EMPTY, NOT_ALLOWED};

/// Validates a document against a schema. Convenience wrapper around
/// [`Validator`] for one-shot use.
pub fn validate(schema: &Schema, doc: &Document, text: &str) -> Vec<ValidationDiagnostic> {
    Validator::new(schema).validate(doc, text)
}

/// Reusable validator. Keeping it alive between runs preserves the memo
/// tables of the derivative machine, which is what makes revalidating a
/// document on every keystroke cheap.
pub struct Validator<'s> {
    machine: Machine<'s>,
}

impl<'s> Validator<'s> {
    pub fn new(schema: &'s Schema) -> Self {
        Self {
            machine: Machine::new(schema),
        }
    }

    pub fn validate(&mut self, doc: &Document, text: &str) -> Vec<ValidationDiagnostic> {
        let mut run = Run {
            machine: &mut self.machine,
            doc,
            text,
            diags: Vec::new(),
        };
        if let Some(root) = doc.root_element() {
            let start = run.machine.convert(run.machine.schema.start());
            run.process_element(start, root);
        }
        // A document without a root element is already a parser error;
        // validation has nothing to add.
        run.diags
    }
}

struct Run<'m, 's, 'd> {
    machine: &'m mut Machine<'s>,
    doc: &'d Document,
    text: &'d str,
    diags: Vec<ValidationDiagnostic>,
}

impl Run<'_, '_, '_> {
    /// Consumes one element in the parent state and returns the state
    /// after it (including recovery on errors).
    fn process_element(&mut self, state: VId, node: NodeId) -> VId {
        let element = self.doc.element(node).expect("node is an element").clone();
        let ns = element.name.namespace.as_deref();
        let local = element.name.local.as_str();

        let qid = self.machine.qname(ns, local);
        let opened = self.machine.start_tag_open(state, qid, ns, local);
        if opened == NOT_ALLOWED {
            self.report_element_not_allowed(state, &element);
            // Recovery: if the schema knows an element of this name, still
            // validate the subtree against it; otherwise look for known
            // elements further down. Nested errors stay visible either
            // way, without cascading noise for the unknown element itself.
            if let Some(known) = self.machine.schema.find_element(ns, local) {
                let content = self.machine.element_content(known);
                let body = self.machine.after(content, EMPTY);
                self.element_body(body, &element, node);
            } else {
                self.validate_orphaned_children(node);
            }
            // The parent state is unchanged: the element is ignored.
            return state;
        }

        self.element_body(opened, &element, node)
    }

    /// Validates attributes, children, and the end tag of an element whose
    /// start tag was already consumed. Returns the state after the end
    /// tag.
    fn element_body(&mut self, opened: VId, element: &Element, node: NodeId) -> VId {
        // --- Attributes ---------------------------------------------------
        let mut state = opened;
        for attribute in &element.attributes {
            // Namespace declarations are not attributes in the data model.
            if attribute.name.namespace.as_deref() == Some(XMLNS_NAMESPACE) {
                continue;
            }
            let ns = attribute.name.namespace.as_deref();
            let local = attribute.name.local.as_str();
            let value = unescape(&attribute.value);
            let next = self.machine.attribute(state, ns, local, &value);
            if next != NOT_ALLOWED {
                state = next;
                continue;
            }
            // Distinguish "unknown attribute" from "bad value": does any
            // expected attribute pattern match the name?
            let by_name = self
                .machine
                .expected_attributes(state)
                .into_iter()
                .find(|&id| {
                    self.machine
                        .schema
                        .attribute_pattern(id)
                        .is_some_and(|a| a.name.matches(ns, local))
                });
            match by_name {
                Some(pattern) => {
                    self.report_invalid_attribute_value(pattern, attribute, &value);
                    // Recover by accepting the attribute despite the value.
                    let next = self.machine.attribute_lenient(state, pattern);
                    if next != NOT_ALLOWED {
                        state = next;
                    }
                }
                None => {
                    self.diags.push(ValidationDiagnostic::error(
                        attribute.name_range,
                        ValidationDiagnosticCode::AttributeNotAllowed,
                        format!(
                            "attribute `{}` is not allowed on element `{}`",
                            attribute.name.as_written(),
                            element.name.as_written()
                        ),
                    ));
                    // Ignore the attribute.
                }
            }
        }

        // --- Close the start tag ------------------------------------------
        let mut closed = self.machine.start_tag_close(state);
        if closed == NOT_ALLOWED {
            let missing = self.missing_required_attributes(state);
            let names: Vec<String> = missing
                .iter()
                .filter_map(|&id| self.machine.schema.attribute_pattern(id))
                .map(|a| render_name(&a.name))
                .collect();
            self.diags.push(ValidationDiagnostic::error(
                element.name_range,
                ValidationDiagnosticCode::MissingAttributes,
                if names.is_empty() {
                    format!(
                        "element `{}` is missing required attributes",
                        element.name.as_written()
                    )
                } else {
                    format!(
                        "element `{}` is missing required attribute{} {}",
                        element.name.as_written(),
                        if names.len() > 1 { "s" } else { "" },
                        join_names(&names)
                    )
                },
            ));
            // Recover: pretend the missing attributes were present.
            let mut recovered = state;
            for &attr in &missing {
                let next = self.machine.attribute_lenient(recovered, attr);
                if next != NOT_ALLOWED {
                    recovered = next;
                }
            }
            closed = self.machine.start_tag_close(recovered);
            if closed == NOT_ALLOWED {
                closed = recovered;
            }
        }

        // --- Children ------------------------------------------------------
        let mut state = closed;
        for &child in self.doc.children(node) {
            match self.doc.kind(child) {
                NodeKind::Element(_) => {
                    state = self.process_element(state, child);
                }
                NodeKind::Text | NodeKind::CData => {
                    let raw = self.child_text(child);
                    let next = self.machine.text(state, &raw);
                    if next == NOT_ALLOWED {
                        let expects_text = self.machine.expects_text(state);
                        self.report_bad_text(expects_text, element, self.doc.range(child), &raw);
                        if expects_text {
                            // Recover as if the value had matched, so the
                            // end tag does not fail a second time.
                            let lenient = self.machine.text_lenient(state);
                            if lenient != NOT_ALLOWED {
                                state = lenient;
                            }
                        }
                        // Otherwise ignore the text entirely.
                    } else {
                        state = next;
                    }
                }
                // Comments, PIs, and doctypes are invisible to validation.
                _ => {}
            }
        }

        // --- End tag -------------------------------------------------------
        let mut ended = self.machine.end_tag(state);
        if ended == NOT_ALLOWED {
            // An element without children may still satisfy a data/value
            // content model with the empty string.
            let with_empty_text = self.machine.text(state, "");
            if with_empty_text != NOT_ALLOWED {
                ended = self.machine.end_tag(with_empty_text);
            }
        }
        if ended == NOT_ALLOWED {
            self.report_incomplete(state, element);
            ended = self.machine.end_tag_forced(state);
            if ended == NOT_ALLOWED {
                // Nothing to salvage; at least do not poison the parent.
                ended = EMPTY;
            }
        }
        ended
    }

    /// Text content of a text or CDATA node, entity references resolved.
    fn child_text(&self, node: NodeId) -> String {
        let range = self.doc.range(node);
        match self.doc.kind(node) {
            NodeKind::CData => {
                let start = (range.start + 9).min(range.end);
                let end = range.end.saturating_sub(3).max(start);
                self.text[start..end].to_string()
            }
            _ => unescape(&self.text[range.start..range.end]).into_owned(),
        }
    }

    /// Greedily determines which required attributes would unblock the
    /// start tag close.
    fn missing_required_attributes(&mut self, state: VId) -> Vec<PatternId> {
        let mut missing = Vec::new();
        let mut current = state;
        // Each round must consume one more attribute; the candidate list
        // is finite, so this terminates.
        loop {
            if self.machine.start_tag_close(current) != NOT_ALLOWED {
                break;
            }
            let candidates = self.machine.expected_attributes(current);
            let mut advanced = false;
            for candidate in candidates {
                let next = self.machine.attribute_lenient(current, candidate);
                if next != NOT_ALLOWED {
                    missing.push(candidate);
                    current = next;
                    advanced = true;
                    break;
                }
            }
            if !advanced {
                break;
            }
        }
        missing
    }

    fn report_element_not_allowed(&mut self, state: VId, element: &Element) {
        let mut expected = self.expected_element_names(state);
        // For typos, the most similar names are the most helpful ones —
        // sort them to the front so they survive the display limit.
        let written = element.name.local.as_str();
        expected.sort_by_key(|name| levenshtein(name.trim_matches('`'), written));
        let message = if expected.is_empty() {
            format!(
                "element `{}` is not allowed here",
                element.name.as_written()
            )
        } else {
            format!(
                "element `{}` is not allowed here; expected {}",
                element.name.as_written(),
                join_names(&expected)
            )
        };
        self.diags.push(ValidationDiagnostic::error(
            element.name_range,
            ValidationDiagnosticCode::ElementNotAllowed,
            message,
        ));
    }

    fn report_invalid_attribute_value(
        &mut self,
        pattern: PatternId,
        attribute: &xml_ast::Attribute,
        value: &str,
    ) {
        let content = self
            .machine
            .schema
            .attribute_pattern(pattern)
            .expect("attribute pattern")
            .content;
        let allowed: Vec<String> = self
            .machine
            .schema
            .value_enum(content)
            .iter()
            .filter_map(|&id| self.machine.schema.value_pattern(id))
            .map(|v| format!("`{}`", v.value.trim()))
            .collect();
        let message = if allowed.is_empty() {
            format!(
                "invalid value `{}` for attribute `{}`",
                value,
                attribute.name.as_written()
            )
        } else {
            format!(
                "invalid value `{}` for attribute `{}`; allowed: {}",
                value,
                attribute.name.as_written(),
                limit_list(&allowed, 10)
            )
        };
        let range = if attribute.value_range.is_empty() {
            attribute.name_range
        } else {
            attribute.value_range
        };
        self.diags.push(ValidationDiagnostic::error(
            range,
            ValidationDiagnosticCode::InvalidAttributeValue,
            message,
        ));
    }

    /// Validates the subtree of an element the schema does not know at
    /// all: known elements found further down are validated standalone.
    fn validate_orphaned_children(&mut self, node: NodeId) {
        for &child in self.doc.children(node) {
            let Some(element) = self.doc.element(child).cloned() else {
                continue;
            };
            let ns = element.name.namespace.as_deref();
            let known = self.machine.schema.find_element(ns, &element.name.local);
            if let Some(known) = known {
                let content = self.machine.element_content(known);
                let body = self.machine.after(content, EMPTY);
                self.element_body(body, &element, child);
            } else {
                self.validate_orphaned_children(child);
            }
        }
    }

    fn report_bad_text(
        &mut self,
        expects_text: bool,
        element: &Element,
        range: TextRange,
        raw: &str,
    ) {
        if expects_text {
            self.diags.push(ValidationDiagnostic::error(
                range,
                ValidationDiagnosticCode::InvalidTextValue,
                format!(
                    "invalid content `{}` in element `{}`",
                    raw.trim(),
                    element.name.as_written()
                ),
            ));
        } else {
            self.diags.push(ValidationDiagnostic::error(
                range,
                ValidationDiagnosticCode::TextNotAllowed,
                format!(
                    "text is not allowed inside element `{}`",
                    element.name.as_written()
                ),
            ));
        }
    }

    fn report_incomplete(&mut self, state: VId, element: &Element) {
        let expected = self.expected_element_names(state);
        let range = element.close_tag.unwrap_or(element.name_range);
        let message = if expected.is_empty() {
            format!(
                "element `{}` is missing required content",
                element.name.as_written()
            )
        } else {
            format!(
                "element `{}` is missing required content; expected {}",
                element.name.as_written(),
                join_names(&expected)
            )
        };
        self.diags.push(ValidationDiagnostic::error(
            range,
            ValidationDiagnosticCode::IncompleteContent,
            message,
        ));
    }

    fn expected_element_names(&mut self, state: VId) -> Vec<String> {
        let mut names: Vec<String> = self
            .machine
            .expected_elements(state)
            .iter()
            .filter_map(|&id| self.machine.schema.element_pattern(id))
            .map(|el| render_name(&el.name))
            .collect();
        names.sort();
        names.dedup();
        names
    }
}

/// Renders a name class for messages: concrete names verbatim, wildcards
/// descriptively.
fn render_name(name: &NameClass) -> String {
    match name {
        NameClass::Name { local, .. } => format!("`{local}`"),
        NameClass::NsName { ns, .. } => match ns {
            Some(ns) => format!("any name from namespace `{ns}`"),
            None => "any unqualified name".to_string(),
        },
        NameClass::AnyName { .. } => "any name".to_string(),
        NameClass::Choice(choices) => {
            let parts: Vec<String> = choices.iter().map(render_name).collect();
            parts.join(" or ")
        }
    }
}

fn join_names(names: &[String]) -> String {
    limit_list(names, 8)
}

/// Classic edit distance, used to rank name suggestions.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut previous_diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitution = previous_diagonal + usize::from(ca != cb);
            previous_diagonal = row[j + 1];
            row[j + 1] = substitution.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[b.len()]
}

fn limit_list(items: &[String], max: usize) -> String {
    if items.len() <= max {
        items.join(", ")
    } else {
        format!(
            "{}, … ({} more)",
            items[..max].join(", "),
            items.len() - max
        )
    }
}
