# Module: storage.rs — Vault Storage Layer

## Purpose
Handles vault paths, directory creation with permissions, atomic file writes, config file access, and YAML frontmatter parsing.

## Key Symbols
- **`Paths`** (struct): `{ vault_dir, granola_dir, baez_dir, raw_dir, summaries_dir, tmp_dir }`
  - `new(vault_override)` — resolves vault: CLI flag → `BAEZ_VAULT` → config file → XDG fallback
  - `doc_path(created_at, slug)` → `granola_dir/YYYY/MM/YYYY-MM-DD_slug.md`
  - `ensure_dirs()` — creates all directories, sets 0o700 on `.baez/` subdirs
- **`write_atomic(path, content, tmp_dir)`** — atomic write via temp file + rename, 0o600 permissions
- **`set_file_time(path, datetime)`** — sets mtime via `filetime` crate
- **`read_frontmatter(md_path)`** → `Option<Frontmatter>` — parses YAML between `---` delimiters
- **`read_config_field(field)`** → reads from `~/.config/baez/config.json`
- **`warn_config_permissions()`** — warns if config file with secrets has loose permissions
- **`config_file_path()`** (private) — resolves XDG_CONFIG_HOME

## Vault Layout
```
Vault/
└── Granola/
    ├── YYYY/MM/YYYY-MM-DD_slug.md
    └── .baez/
        ├── raw/                (transcript + metadata JSON)
        ├── summaries/          (AI summary files)
        ├── tmp/                (atomic write temp dir)
        ├── summary_config.json
        └── .sync_cache.json
```

## Security
- `.baez/` directories: 0o700 (owner only)
- Written files: 0o600 (owner only)
- Config file permission warning when containing secrets
