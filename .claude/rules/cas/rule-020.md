---
id: rule-020
paths: "lib/**/*_controller.ex"
---

ALWAYS use render_inertia/3 and pass tuples {Serializer, data} - never call NbSerializer.serialize!() directly