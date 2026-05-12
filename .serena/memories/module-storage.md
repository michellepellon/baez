# Module: storage.rs — Vault Storage Layer + Knowledge Graph Notes

## Purpose
Handles vault paths, directory creation with permissions, atomic file writes, config file access, YAML frontmatter parsing, AND knowledge-graph entity note CRUD (People, Concepts, Projects) with fuzzy name matching.

## Key Symbols
- **`Paths`** (struct): `{ vault_dir, granola_dir, baez_dir, raw_dir, summaries_dir, tmp_dir }`
  - `new(vault_override)` — resolves vault: CLI flag → `BAEZ_VAULT` → config file → XDG fallback
  - `doc_path(created_at, slug)` → `granola_dir/YYYY/MM/YYYY-MM-DD_slug.md`
  - `ensure_dirs()` — creates all directories, sets 0o700 on `.baez/` subdirs (does NOT create People/Concepts/Projects — those happen in `sync.rs` only when summarization is active)
- **`write_atomic(path, content, tmp_dir)`** — atomic write via temp file + rename, 0o600 permissions
- **`set_file_time(path, datetime)`** — sets mtime via `filetime` crate
- **`read_frontmatter(md_path)`** → `Option<Frontmatter>` — strict parse into `Frontmatter` struct
- **`read_entity_frontmatter(path)`** → `Option<(serde_json::Value, String)>` — flexible parse (returns YAML as Value + body separately) for entity notes whose frontmatter shape varies
- **`find_entity_file(dir, name)`** → `Option<PathBuf>` — case-insensitive lookup by filename
- **`read_config_field(field)`** → reads from `~/.config/baez/config.json`
- **`warn_config_permissions()`** — warns if config file with secrets has loose permissions
- **`config_file_path()`** (private) — resolves XDG_CONFIG_HOME

### Knowledge Graph (PeopleIndex)
- **`PeopleIndex`** (struct): In-memory index of `People/` notes for fuzzy name matching
  - `entries: HashMap<lowercase_name, (original_case, path)>`
  - `aliases: HashMap<lowercase_alias, lowercase_canonical>`
  - `build(people_dir)` — scans `*.md`, parses YAML `aliases` arrays into the alias map
  - `add_person(name, people_dir, new_aliases)` — register newly created person (no I/O)
  - `find_match(name, attendees)` → `Option<(canonical, path)>` — four-step lookup:
    1. Exact (lowercase)
    2. Alias
    3. First-name disambiguation against attendee list (single match wins)
    4. Levenshtein ≤ 2, skipped for names ≤ 5 chars, ambiguous → None

### Entity Note CRUD
- **`create_person_note(...)`** — initial People note with frontmatter (`title`, `aliases`, `type: person`, `role`, `company`, `last-contact`, `status: active`, `related`), `## Context`, `## Notes` sections
- **`enrich_person_note(...)`** — adds meeting ref to `related`, updates `last-contact` if newer, merges aliases case-insensitively, appends bullet to `## Notes`
- **`create_concept_note(...)`** — initial Concept note with `type: concept`, `## What is it?`, `## Sources`
- **`enrich_concept_note(...)`** — adds meeting ref to `related`, appends source line to `## Sources`
- **`create_project_note(...)`** — initial Project note with `type: project`, body line `Mentioned in [[meeting]]: description`
- **`enrich_project_note(...)`** — adds meeting ref to `related`, appends another mention line
- **`merge_frontmatter_related(md_path, new_links, tmp_dir)`** — set-union merge of `related:` array in an existing markdown file's YAML frontmatter. Hand-rolled string manipulation (preserves field order); skips write when no new links.
- **`parse_aliases_from_frontmatter(content)`** (private) — extracts `aliases` array from YAML

## Vault Layout
```
Vault/
├── People/                  Auto-created during sync when summarize+key present
│   └── Alice Smith.md
├── Concepts/
│   └── API Design.md
├── Projects/
│   └── Project Atlas.md
└── Granola/
    ├── YYYY/MM/YYYY-MM-DD_slug.md
    └── .baez/
        ├── raw/                (transcript + metadata JSON)
        ├── summaries/          (legacy summary files dir)
        ├── tmp/                (atomic write temp dir)
        ├── summary_config.json
        ├── .sync_cache.json
        └── .summary_cache.json
```

## Security
- `.baez/` directories: 0o700 (owner only)
- Written files: 0o600 (owner only)
- Config file permission warning when containing secrets
- Entity notes (People/Concepts/Projects) inherit `write_atomic`'s 0o600

## Test Modules
- `tests`, `write_tests`, `frontmatter_tests`, `people_index_tests`, `entity_note_tests`
