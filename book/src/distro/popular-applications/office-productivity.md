# Office & Productivity

This category covers office suites, document viewers, and note-taking applications.

## Office Suites

### TODO: LibreOffice

[LibreOffice](https://www.libreoffice.org/) is a powerful open-source office suite compatible with major office file formats.

## Document Viewers

### TODO: Evince

[Evince](https://wiki.gnome.org/Apps/Evince) is a document viewer for PDF, PostScript, and other document formats.

### TODO: Okular

[Okular](https://okular.kde.org/) is a universal document viewer developed by KDE.

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

### TODO: Zathura

[Zathura](https://pwmt.org/projects/zathura/) is a highly customizable document viewer with vi-style keybindings.

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

## Note-Taking & Knowledge Management

### TODO: Obsidian

[Obsidian](https://obsidian.md/) is a note-taking app that works on local Markdown files.

### TODO: Joplin

[Joplin](https://joplinapp.org/) is an open-source note-taking and to-do application.
