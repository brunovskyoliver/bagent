---
status: accepted
---

# Use session-and-origin submission grants

Ordinary browser interactions on allowlisted destinations run automatically, but form submission requires one user-approved Submission Grant for that Browser Session and origin. The grant covers all submissions on that origin because an embedded browser cannot reliably classify a submission as harmless or destructive from page-provided semantics. Password fields remain human-only. This differs from bagent's usual per-write approval model and must be stated plainly when the user grants permission.
