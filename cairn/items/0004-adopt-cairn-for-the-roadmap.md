---
id: 4
title: Adopt cairn for the roadmap
type: feature
status: done
milestone: v0.1
created: 2026-09-08
updated: 2026-09-08
---

This is a cairn item: a Markdown file with YAML frontmatter. Edit it by hand,
or from the command line:

    cairn set 1 status=done
    cairn show 1
    cairn list --status done

Delete this file once you have the hang of it.

## Acceptance criteria

- [ ] `cairn.toml` describes the workflow this project actually uses
- [ ] `cairn render` produces a ROADMAP.md worth linking from the README
- [ ] `cairn check` passes in CI
