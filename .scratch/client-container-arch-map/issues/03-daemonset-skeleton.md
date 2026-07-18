# Ticket 03: Produce DaemonSet + pod-security skeleton manifest

Label: `wayfinder:task`
Status: open
Blocked by: 01, 02

## Question

Task (unblocks implementation): author a minimal, commented Kubernetes DaemonSet
manifest for the client container, consistent with tickets 01 (device mounts +
securityContext) and 02 (rule-engine config mount + hot-reload topic).

Resolve when: a `daemonset.yaml` exists with — DaemonSet spec pinning one pod per
node, the device mounts from 01, the `securityContext` posture from 01, a
ConfigMap/volume for the rule config, the zenoh hot-reload topic wired, and
liveness/readiness probe stubs (endpoint defined in 04). Document each field's
purpose. This is a skeleton for review, not production-hardened.

No decision here — the decisions are 01/02; this just renders them as a manifest.
