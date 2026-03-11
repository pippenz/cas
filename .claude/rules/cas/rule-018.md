---
id: rule-018
paths: "lib/**/*_controller.ex"
---

Use :inertia_controller not :controller for Inertia pages. This prevents using json(conn, data) which would break Inertia - it raises a compile-time error if attempted.