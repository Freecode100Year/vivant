# README Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite both GitHub READMEs to invite users into VIVANT first and present self-hosting as a clear secondary path.

**Architecture:** Documentation-only change. Keep the Chinese and English files parallel in structure; source feature and privacy claims from existing docs.

**Tech Stack:** Markdown.

**Spec:** `docs/superpowers/specs/2026-11-07-readme-rewrite-design.md`

## Global Constraints

- Preserve valid repository and documentation links.
- Keep the stated privacy boundary explicit: the server can read chat messages.
- Do not change code or other product documentation.

## Review Focus

- Chinese and English versions describe the same join, privacy, and self-hosting paths.
- Public connection command and deployment links remain valid.
- No completed roadmap checklist or unsupported claims remain.

---

### Task 1: Rewrite the bilingual GitHub README

**Files:**
- Modify: `README.md`
- Modify: `README.en.md`

**Interfaces:**
- Consumes: Existing usage, security, agent, and deployment documentation.
- Produces: Parallel, join-first READMEs with a concise product introduction, SSH quick start, feature summary, explicit privacy boundary, secondary self-hosting entry, and documentation links.

- [ ] Rewrite `README.md` and `README.en.md` in matching order; retain the supported public SSH endpoint and existing docs links, remove the completed plan list, and keep deployment details in the deployment guide.
- [ ] Review `git diff --check`, compare section structure between languages, and verify each README link target exists.
- [ ] Commit the reviewed documentation changes on the feature branch; do not publish them to the shared repository yet.
