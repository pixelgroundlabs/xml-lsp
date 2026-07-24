# Zed extension for xml-lsp

Registers the `xml-lsp` language server with Zed. Zed only launches
language servers that an extension registers — pointing `settings.json`
at an arbitrary binary is not enough (that is why Zed flags
`"xml-lsp"` under `lsp` as "not allowed" without this extension).

## Installation (dev extension)

1. Build the language server:

   ```sh
   cargo build --release   # in the repository root
   ```

2. Zed compiles dev extensions itself and needs a **rustup-managed**
   toolchain with the WASM target (current Zed builds extensions for
   `wasm32-wasip2`):

   ```sh
   rustup target add wasm32-wasip2
   ```

   If `cargo` resolves to a Homebrew-installed Rust (`which cargo` →
   `/opt/homebrew/bin/cargo`), the build fails with "target may not be
   installed" even though the target exists — put the rustup proxies
   first in your PATH:

   ```sh
   export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
   ```

3. In Zed, install the **XML** extension from the extension gallery
   (it provides the XML language and grammar this extension attaches
   to).

4. In Zed: `zed: extensions` → **Install Dev Extension** → select this
   directory (`editors/zed`).

5. Configure the server in your `settings.json` (the `xml-lsp` entry
   becomes valid once the extension is installed):

   ```json
   {
     "lsp": {
       "xml-lsp": {
         "binary": {
           "path": "/path/to/lsp-speedata/target/release/xml-lsp"
         },
         "initialization_options": {
           "schema": {
             "catalogs": [
               "/path/to/publisher/schema/catalog-schema-de.xml"
             ]
           }
         }
       }
     }
   }
   ```

   Without `binary.path`, the extension looks for `xml-lsp` on `PATH`.

6. Open a `layout.xml` — validation diagnostics and completion
   (elements, attributes, values, each with schema documentation)
   should appear. `cmd-shift-p` → `debug: open language server logs`
   shows the server's log if something is off.

## Notes

- The extension passes `initialization_options` through verbatim; all
  server options are documented in the repository README.
- The German/English documentation variant follows the catalog or
  schema you configure (`catalog-schema-de.xml` vs. `-en.xml`).
