# Explicit Repair Records Gate BYOP Placeholder Tool Results

Status: retired by #152; retained only as historical design context. The Custom-Provider HTTP
transport and its repair path are no longer part of the Zaplex product.

Normal BYOP request flow must not fabricate placeholder tool results when a model tool call is missing its recorded result. Zap will block normal-flow serialization until the tool call has a real terminal result, while placeholder tool results remain available only for explicit history repair backed by per-tool-call Repair Records.

The rejected alternative was to keep the broad sanitizer behavior and infer repair from a missing result. That keeps provider payloads protocol-valid, but it lets the model reason from an artificial observation and hides timing, persistence, fork, or restore bugs. Requiring explicit Repair Records makes repair rarer and more work to set up, but it keeps normal agent execution grounded in real results and leaves a durable explanation for every placeholder that is intentionally emitted.
