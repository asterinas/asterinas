# Office & Productivity

This category covers office suites, document viewers, and note-taking applications.

## Document Viewers

### MuPDF

[MuPDF](https://mupdf.com/) is a lightweight PDF and XPS viewer.

#### Installation

```nix
environment.systemPackages = [ pkgs.mupdf ];
```

#### Verified Usage

```bash
# Show information about pdf resources
mutool info file.pdf

# Convert text from pdf
mutool draw -F text -o - file.pdf

# Convert images from pdf
mutool draw -F png -o page-%03d.png sample.pdf
```

### Pandoc

[Pandoc](https://hackage.haskell.org/package/pandoc-cli) is a universal document converter.

#### Installation

```nix
environment.systemPackages = [ pkgs.pandoc ];
```

#### Verified Usage

```bash
# Convert Markdown to HTML
pandoc test.md -o test.html

# Convert Markdown to Word DOCX
pandoc test.md -o test.docx

# Convert HTML to Markdown
pandoc test.html -f html -t markdown -o converted.md
```
