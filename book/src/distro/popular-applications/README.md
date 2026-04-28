# Popular Applications

The [Nix package collection](https://search.nixos.org/packages) contains over 120,000 packages.
Many of them may already work out of the box on Asterinas NixOS,
but it is impractical to test every package
and document its support status.
Instead, we group popular applications into categories
and document the ones we have verified to work.

## Categories

The categorization is designed to scale to hundreds of applications while serving both server and desktop use cases.
It follows a two-level hierarchy (Category > Sub-category) with the following top-level categories:

| Category | Description | Packages |
|----------|-------------|----------|
| [System Core](system-core/README.md) | Shells, init systems, system monitoring, and essential utilities | "Bash", "Fish", "Zsh", "BusyBox", "Systemd", "Fastfetch", "Htop", "Lsof", "Ncdu", "Procps", "Coreutils", "Diffutils", "Findutils", "Grep", "Hostname", "Less", "Man-pages", "Texinfo", "Util-linux", "Which" |
| [Nix and NixOS Tools](nix-and-nixos-tools/README.md) | Nix package management and NixOS system management tools | "Nix" |
| [Containerization and Virtualization](containerization-and-virtualization/README.md) | Container runtimes and image management tools | "Podman", "Skopeo" |
| [Networking](networking/README.md) | Network utilities, DNS, VPN, and firewalls | "Curl", "LFTP", "Netcat", "Rclone", "Rsync", "Socat", "Wget", "LDNS", "Whois" |
| [Web Servers & Proxies](web-servers-and-proxies/README.md) | Web servers and reverse proxies | "Apache HTTP Server", "Caddy", "Nginx", "OpenResty" |
| [Databases & Middleware](databases-and-middleware/README.md) | Relational databases, NoSQL, search engines, and message queues | "SQLite", "Etcd", "Redis", "Valkey", "InfluxDB" |
| [Development Tools](development-tools/README.md) | Language runtimes, build tools, editors, and debugging tools | "Clang", "GCC", "Go", "Lua", "Node.js", "Octave", "OpenJDK", "Perl", "PHP", "Python3", "Ruby", "Rust", "Git", "Cargo", "CMake", "Make", "Meson", "Ninja", "Emacs", "Nano", "Neovim", "Vim", "Hugo", "Direnv", "ShellCheck", "jq", "yq" |
| [CI/CD & DevOps](cicd-and-devops/README.md) | CI/CD runners and infrastructure automation | "Just", "Task", "GoReleaser" |
| [Monitoring & Observability](monitoring-and-observability/README.md) | Metrics, logging, and tracing tools | "Prometheus" |
| [Desktop Environments & Display](desktop-environments-and-display/README.md) | Desktop environments, window managers, and display servers | "Xfce", "galculator", "mousepad", "mupdf", "fairymax", "five-or-more", "lbreakout2", "gnome-chess", "gnome-mines", "gnome-sudoku", "tali", "xboard", "xgalaga" |
| [Web Browsers](web-browsers/README.md) | Web browsers | "Links2", "W3m" |
| [Office & Productivity](office-and-productivity/README.md) | Office suites, document viewers, and note-taking apps | "MuPDF", "Pandoc" |
| [Multimedia](multimedia/README.md) | Video/audio players, graphics tools, and streaming software | "SoX", "ImageMagick", "FFmpeg" |
| [Communication](communication/README.md) | Email clients, instant messaging, and video conferencing | "Irssi", "WeeChat" |
| [File Management & Terminal](file-management-and-terminal/README.md) | File managers, terminal emulators, archives, and CLI utilities | "Bzip2", "Gzip", "P7zip", "Tar", "Xz", "Zip", "Screen", "File", "Bat", "Gawk", "Sd", "Sed", "Eza", "Fd", "Fzf", "Ripgrep", "The Silver Searcher", "Tree", "Age", "Crunch", "GnuPG", "John the Ripper", "Restic", "Wipe" |
| [AI & Machine Learning](ai-and-machine-learning/README.md) | Deep learning frameworks, LLM tools, and inference engines | "PyTorch", "TensorFlow", "Ollama" |
