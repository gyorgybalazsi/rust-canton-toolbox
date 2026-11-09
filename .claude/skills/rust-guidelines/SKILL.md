---
name: rust-guidelines
description: Microsoft's Pragmatic Rust Guidelines — idiomatic, scalable Rust API/design rules. Use when writing or reviewing Rust, designing a public API or crate boundary, choosing types vs generics vs dyn, modeling errors, adding unsafe, documenting items, or making performance/logging/AI-ergonomics decisions. Complements the team's rust-coding-standards (house style) with upstream design guidance.
---

# Pragmatic Rust Guidelines (Microsoft)

Design guidance for idiomatic Rust that scales, from
<https://microsoft.github.io/rust-guidelines/>. Builds on the official Rust API
Guidelines. Items are **M-** ("must" — required) or should-level (flexible);
the checklist below is the must-level backbone. Read the source page for the
full rationale, examples, and should-level items before applying any rule to a
non-obvious case.

When reviewing or writing Rust here, check the relevant section. Cite the rule
ID (e.g. `M-DI-HIERARCHY`) when flagging something so it's traceable.

> Scope note for this repo: the **Universal**, **Safety**, **Performance**, and
> **Documentation** sections apply broadly. **Libraries** items apply to the
> reusable crates (`ledger-api`, `client`, `daml-type-rep`, `codegen`,
> `submit`). **FFI** is largely N/A. Generated code (`ledger-api/src/pb`) is
> exempt — don't retrofit guidelines onto it.

## Universal
- `M-UPSTREAM-GUIDELINES` — Follow the upstream Rust API Guidelines.
- `M-STATIC-VERIFICATION` — Use static verification (clippy, etc.).
- `M-LINT-OVERRIDE-EXPECT` — Lint overrides should use `#[expect]` (not bare `#[allow]`) so they fail when no longer needed.
- `M-PUBLIC-DEBUG` — Public types are `Debug`.
- `M-PUBLIC-DISPLAY` — Public types meant to be read are `Display`.
- `M-SMALLER-CRATES` — If in doubt, split the crate.
- `M-CONCISE-NAMES` — Names are free of weasel words.
- `M-REGULAR-FN` — Prefer regular over associated functions.
- `M-PANIC-IS-STOP` — Panic means "stop the program."
- `M-PANIC-ON-BUG` — Detected programming bugs are panics, not errors.
- `M-DOCUMENTED-MAGIC` — All magic values and behaviors are documented.
- `M-LOG-STRUCTURED` — Use structured logging with message templates.

## Libraries — Interoperability
- `M-TYPES-SEND` — Types are `Send`.
- `M-ESCAPE-HATCHES` — Provide native escape hatches.
- `M-DONT-LEAK-TYPES` — Don't leak external types in your public API.

## Libraries — UX
- `M-SIMPLE-ABSTRACTIONS` — Abstractions don't visibly nest.
- `M-AVOID-WRAPPERS` — Avoid smart pointers and wrappers in APIs.
- `M-DI-HIERARCHY` — Prefer types over generics, generics over `dyn` traits.
- `M-ERRORS-CANONICAL-STRUCTS` — Errors are canonical structs.
- `M-INIT-BUILDER` — Complex type construction has builders.
- `M-INIT-CASCADED` — Complex type initialization hierarchies are cascaded.
- `M-SERVICES-CLONE` — Services are `Clone`.
- `M-IMPL-ASREF` — Accept `impl AsRef<>` where feasible.
- `M-IMPL-RANGEBOUNDS` — Accept `impl RangeBounds<>` where feasible.
- `M-IMPL-IO` — Accept `impl 'IO'` where feasible ("Sans IO").
- `M-ESSENTIAL-FN-INHERENT` — Essential functionality should be inherent.

## Libraries — Resilience
- `M-MOCKABLE-SYSCALLS` — I/O and system calls are mockable.
- `M-TEST-UTIL` — Test utilities are feature-gated.
- `M-STRONG-TYPES` — Use the proper type family.
- `M-NO-GLOB-REEXPORTS` — Don't glob re-export items.
- `M-AVOID-STATICS` — Avoid statics.

## Libraries — Building
- `M-OOBE` — Libraries work out of the box.
- `M-SYS-CRATES` — Native `-sys` crates compile without dependencies.
- `M-FEATURES-ADDITIVE` — Features are additive.

## Applications
- `M-MIMALLOC-APP` — Use mimalloc for apps.
- `M-APP-ERROR` — Applications may use `anyhow` or derivatives.

## FFI
- `M-ISOLATE-DLL-STATE` — Isolate DLL state between FFI libraries.

## Safety
- `M-UNSAFE` — Unsafe needs a reason and should be avoided.
- `M-UNSAFE-IMPLIES-UB` — Unsafe implies undefined behavior (treat it that seriously).
- `M-UNSOUND` — All code must be sound.

## Performance
- `M-THROUGHPUT` — Optimize for throughput, avoid empty cycles.
- `M-HOTPATH` — Identify, profile, and optimize the hot path early.
- `M-YIELD-POINTS` — Long-running tasks should have yield points.

## Documentation
- `M-FIRST-DOC-SENTENCE` — First sentence is one line; approx. 15 words.
- `M-MODULE-DOCS` — Has module documentation.
- `M-CANONICAL-DOCS` — Documentation has canonical sections.
- `M-DOC-INLINE` — Mark `pub use` items with `#[doc(inline)]`.

## AI
- `M-DESIGN-FOR-AI` — Design with AI use in mind.

---

Source: Microsoft Pragmatic Rust Guidelines — <https://microsoft.github.io/rust-guidelines/>
(checklist: <https://microsoft.github.io/rust-guidelines/guidelines/checklist/index.html>).
This skill summarizes the must-level checklist; consult the source for full
rationale, examples, and should-level guidance.
