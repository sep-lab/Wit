# Architecture decision records

One file per decision: the context, the decision, the consequences, and — importantly —
**what evidence would overturn it.**

| # | Decision | Status |
|---|---|---|
| [0001](0001-version-the-recipe-not-the-render.md) | Version the recipe, not the render | Accepted |
| [0002](0002-storage-model.md) | Delta chains for projects; content addressing for audio | Accepted |
| [0003](0003-plugin-state-policy.md) | Track plugin state opaquely; never interpret or port it | Accepted |
| [0004](0004-implementation-stack.md) | Rust core, Python for research | Accepted |
| [0005](0005-first-daw-target.md) | Ableton Live is the first DAW target | Accepted |

These are settled. Reopening one is welcome, but bring the evidence its
"What would overturn this" section asks for — that is what the section is for.

New ADRs: copy the structure of an existing one, take the next number, and link it here
and from [AGENTS.md](../../AGENTS.md) if it constrains how agents should work.
