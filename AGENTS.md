# AGENTS.md for `regroup`

## Codebase Exploration Skills

Use these commands to explore the codebase efficiently:

### 1. Get Directory Tree

Shows the complete project structure with directories listed first for better readability.

```bash
tree -I ".git|target" --dirsfirst
```

### 2. List Public Symbols

Shows all public functions, structs, enums, modules, consts, and use statements, grouped by module with module path as header.

```bash
for module in src/*/; do
  [ -d "$module" ] && \
  echo "" && \
  echo "${module#src/}" && \
  grep -h -E '^pub (fn |struct |enum |mod |use |const )' "$module"/*.rs 2>/dev/null | \
  sed 's/^  //' | \
  sort -u; 
done
```

### 3. Show Module Dependencies

Shows **explicit** cross-module usage patterns (via `use crate::`), grouped by source module with module path as header and blank line between each module. Filters out comments, code, and empty lines.

**Note:** This only captures dependencies declared with `use crate::`. Module children declared in `mod.rs` can be used directly without `use` statements, so the actual dependency graph may include additional implicit edges.

```bash
for file in src/**/*.rs; do
  [ -f "$file" ] && \
  module=$(echo "$file" | sed -E 's|^src/||; s|\.rs$||; s|/mod$||') && \
  deps=$(awk '/^use crate::/ {
    flag=1
    print
    next
  }
  flag {
    if (/^[^ \t]/ && !/^\/\// && !/^$/) {
      flag=0
      next
    }
    print
  }' "$file" | sed -E 's|^ *use crate::||; s|;||; /^ *$/d; /^\/\//d') && \
  [ -n "$deps" ] && \
  echo "$module:" && \
  echo "$deps" | sed 's|^|  └── |' && \
  echo ""
done
```

---

## Language

All code, commits, and documentation must be in English. This is an English-language repository.
