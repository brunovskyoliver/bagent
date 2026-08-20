---
status: accepted
---

# Own the WebKit browser and its profile

bagent Browser is a bagent-owned WebKit browser, not a controlled or embedded Safari.app window. It keeps cookies and other website data in a dedicated persistent Browser Profile and never reads from or writes to the user's Safari profile. This permits the hidden chromeless popup while keeping agent browsing separate from personal Safari data. Users must sign in again inside bagent Browser, and Safari history, AutoFill, extensions, and existing sessions are unavailable there.
