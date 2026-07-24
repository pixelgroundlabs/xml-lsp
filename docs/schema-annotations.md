# Schema annotations (`urn:xml-lsp:annotations`)

xml-lsp carries no language-specific knowledge. Everything beyond what
RELAX NG itself expresses — which attributes define and reference named
symbols, and how the formatter should treat certain elements — comes
from annotations embedded in the schema. RELAX NG explicitly allows
foreign-namespace elements inside patterns (the same mechanism as
`a:documentation`), so annotated schemas remain valid for every other
RELAX NG tool.

Declare the namespace once on the grammar root:

```xml
<grammar xmlns="http://relaxng.org/ns/structure/1.0"
         xmlns:lsp="urn:xml-lsp:annotations">
```

Malformed annotations (unknown element, missing `symbol`, unknown
`form`) produce `schema-annotation` warnings when the schema loads;
the schema itself keeps working.

## Symbol references: `lsp:defines` / `lsp:references`

Placed as a **direct child of an `<attribute>` pattern**. They declare
that this attribute's value names a symbol:

```xml
<element name="DefineColor">
  <attribute name="name">
    <lsp:defines symbol="color"/>
  </attribute>
</element>

<element name="Paragraph">
  <optional>
    <attribute name="color">
      <lsp:references symbol="color"/>
    </attribute>
  </optional>
</element>
```

This powers go-to-definition, find-references, rename, symbol
completion (references complete to all defined names in the document),
and semantic highlighting (symbol values as variables, definitions
with the `definition` modifier).

Attributes:

* **`symbol`** (required) — the symbol kind. Definitions and
  references only match within the same kind; a color named `x` and a
  text format named `x` are distinct symbols.
* **`form`** (optional) — how the attribute value carries the name:
  * `literal` (default) — the value is the name itself.
  * `xpath-string` — the value is an XPath expression; only a quoted
    string literal (`select="'chapter1'"`) is indexed, anything else
    is ignored.
  * `xpath-variable` — the value is an XPath expression; only a plain
    variable reference (`select="$counter"`) is indexed. The `$` is
    excluded from the symbol name, so renames work on the bare name.

Notes:

* One annotation per attribute pattern; if several are present, the
  first wins (with a warning).
* An annotation on an attribute inside a shared `<define>` applies to
  **every** element that references the define — annotate shared
  attribute definitions once.
* Wildcard names (`anyName`, `nsName`) cannot anchor symbol sites and
  are ignored.

## Formatter conventions: `lsp:format`

Placed as a **direct child of an `<element>` pattern**. Any subset of
the three boolean attributes:

```xml
<element name="Value">
  <lsp:format preserve="true"/>
  ...
</element>
```

* **`preserve="true"`** — the element's content is emitted verbatim
  (whitespace preserved); nothing inside is reformatted.
* **`blank-lines="true"`** — the element's direct children are
  separated into blank-line groups, like top-level siblings.
* **`inline="true"`** — below this element, text-only elements stay on
  a single line (`<p>text</p>` instead of three lines); text flow is
  treated like HTML inline content.

## Annotating the speedata schemas (guide for the sphelper session)

The layout schemas (`layoutschema-en.rng` / `-de.rng`) are generated
from `commands.xml` by sphelper. To restore navigation, symbol
completion, rename, and the formatter conventions for speedata layouts,
the generator must emit the annotations below. If `commands.xml`
already models "this attribute refers to a defined X", derive the
annotations from that; otherwise encode this table in the generator.

Symbol sites (attribute → annotation):

| Element(s) | Attribute | Annotation |
|---|---|---|
| `DefineColor` | `name` | `<lsp:defines symbol="color"/>` |
| any | `color`, `background-color`, `background-textcolor`, `bordercolor`, `border-color`, `border-top-color`, `border-right-color`, `border-bottom-color`, `border-left-color`, `defaultcolor`, `framecolor`, `hyperlinkbordercolor` | `<lsp:references symbol="color"/>` |
| `DefineColorprofile` | `name` | `<lsp:defines symbol="colorprofile"/>` |
| any | `colorprofile` | `<lsp:references symbol="colorprofile"/>` |
| `DefineFontfamily` | `name` | `<lsp:defines symbol="fontfamily"/>` |
| `DefineFontalias` | `alias` | `<lsp:defines symbol="fontfamily"/>` |
| `DefineFontalias` | `existing` | `<lsp:references symbol="fontfamily"/>` |
| any | `fontfamily` | `<lsp:references symbol="fontfamily"/>` |
| `LoadFontfile` | `name` | `<lsp:defines symbol="fontfile"/>` |
| `Regular`, `Bold`, `Italic`, `BoldItalic` | `fontface` | `<lsp:references symbol="fontfile"/>` |
| `DefineTextformat` | `name` | `<lsp:defines symbol="textformat"/>` |
| any | `textformat` | `<lsp:references symbol="textformat"/>` |
| `DefineGraphic` | `name` | `<lsp:defines symbol="graphic"/>` |
| any | `graphic` | `<lsp:references symbol="graphic"/>` |
| `DefineMatter` | `name` | `<lsp:defines symbol="matter"/>` |
| any | `matter` | `<lsp:references symbol="matter"/>` |
| `PositioningArea` | `name` | `<lsp:defines symbol="area"/>` |
| any | `area` | `<lsp:references symbol="area"/>` |
| `Group` | `name` | `<lsp:defines symbol="group"/>` |
| any | `groupname` | `<lsp:references symbol="group"/>` |
| `Pagetype` | `name` | `<lsp:defines symbol="pagetype"/>` |
| any | `pagetype` | `<lsp:references symbol="pagetype"/>` |
| `Mark` | `select` | `<lsp:defines symbol="mark" form="xpath-string"/>` |
| `A` | `link` | `<lsp:references symbol="mark"/>` |
| `SetVariable`, `Loop` | `variable` | `<lsp:defines symbol="variable"/>` |
| any *except `Mark`* | `select` | `<lsp:references symbol="variable" form="xpath-variable"/>` |

"any" means: every element whose attribute carries that name — in the
generated schema each `<attribute name="…">` occurrence gets the
annotation. `Mark/@select` keeps its `defines` annotation only (one
annotation per attribute; the variable reference does not apply
there).

Formatter hints (element → annotation):

| Element | Annotation |
|---|---|
| `Value` | `<lsp:format preserve="true"/>` |
| `Section` | `<lsp:format blank-lines="true"/>` |
| `HTML` | `<lsp:format inline="true"/>` |

Checklist for the sphelper change:

1. Emit `xmlns:lsp="urn:xml-lsp:annotations"` on the `<grammar>` root.
2. Emit the annotations from the tables above — in **both** language
   variants (they come from the same `commands.xml`).
3. Regenerate `layoutschema-en.rng` and `layoutschema-de.rng` and open
   a layout in an editor running xml-lsp: go-to-definition on a
   `color=` value, value completion, rename, and `<Value>` formatting
   must work again. Schema warnings with code `schema-annotation`
   point at malformed annotations.
