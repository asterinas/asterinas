# Security persona

**Review section:** Security
**Remit:** Could an adversary breach the kernel's security?

**Guideline index (rules supplied by the catalog):**
`book/src/to-contribute/coding-guidelines/for-security/README.md`

**Concerns, in order:**

1. **`unsafe` soundness** — `justify-unsafe-use` (a `// SAFETY:` comment on every `unsafe` block, and the justification must actually hold),
   `document-safety-conds` (a `# Safety` section on every `unsafe` fn/trait),
   `deny-unsafe-kernel` (only OSTD crates may use `unsafe`), `module-boundary-safety`.
   Treat a removed or weakened invariant that an `unsafe` block relies on (e.g. a struct's size or alignment) as a soundness defect
   even if the `unsafe` block itself is untouched.
2. **Validation of untrusted input at trust boundaries**
   — `validate-at-boundaries`: user-supplied data (syscall arguments, user buffers, lengths) must be validated at the boundary,
   then trusted internally.
   A silent clamp/truncation of a user-supplied length that hides an error the contract requires is a defect.
   Check independent arguments separately; one validated flag or length does not
   validate the rest. Trace absent metadata through gates before its fallback.
3. **Exploitable concurrency** — use-after-free, time-of-check/time-of-use.
   Identify the mutable object, actual writers, read snapshots and synchronization;
   distinguish shared state from copies. Check whether validation and the final
   privilege commit use one consistent snapshot of mutable metadata.
4. **Permission transformations.** Establish the expected result from the
   applicable contract, then trace the same input through gates, transformations
   and the final permissions. Choose cases that distinguish branches: ordinary
   vs special identity, absent/empty/nonempty metadata, flags set vs cleared.
   Combine dimensions where the code couples them; compare related capability
   sets for the same case. Check both unauthorized grants and unintended denials.
5. **Security-state encapsulation.** Trace all exposed mutation paths, including
   mutable/atomic getters and raw setters. Check whether callers can bypass
   authorization, validation or required propagation to dependent state.
