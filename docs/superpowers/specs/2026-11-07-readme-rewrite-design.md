# README Rewrite Design

## Intent

Rewrite the Chinese and English GitHub READMEs to help prospective visitors join the public chat quickly, while keeping self-hosting easy to discover.

## Goals and non-goals

- Lead with the chat's purpose and a copyable SSH command.
- Summarize features and privacy accurately, including that the server can read messages.
- Keep self-hosting as a secondary path and link to the existing deployment guide.
- Keep `README.md` and `README.en.md` aligned in structure and meaning.
- Do not change code or other product documentation.

## Approaches

- Join-first README with concise feature/privacy sections and a self-hosting link (chosen; best matches the public project's immediate user action).
- Self-host-first installation guide (rejected; deployment details already have a dedicated guide).

## Constraints

- Preserve valid repository/document links and supported SSH commands.
- Remove the completed plan checklist instead of adding a new roadmap.
- Do not introduce unsupported privacy, performance, or feature claims.
