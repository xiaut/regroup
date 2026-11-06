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

Shows all cross-module usage patterns, grouped by source module with module path as header and blank line between each module.

```bash
grep -rn "use crate::" src/ --include="*.rs" | awk -F'use crate::' '{
    gsub(/:.*$/, "", $1);
    gsub(/\.rs$/, "", $1);
    module = $1;
    sub(/.*\//, "", module);
    deps[module] = deps[module] ? deps[module] "\n  " $2 : "  " $2;
  }
  END {
    n = asorti(deps, sorted);
    for (i = 1; i <= n; i++) {
      print sorted[i];
      print deps[sorted[i]];
      print "";
    }
  }'
```

---

## Language

All code, commits, and documentation must be in English. This is an English-language repository.
