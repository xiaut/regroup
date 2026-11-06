# AGENTS.md for `regroup`

**Immediately execute this command to get the project context:**
```bash
tree -I ".git|target" --dirsfirst
```

Do not start any task without first obtaining the current repository structure via this command.

---

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

Shows all cross-module usage patterns, grouped by source module with module path as header and blank line between each module. Filters out comments, code, and empty lines.

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
