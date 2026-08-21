# Prompt Debug Logs (retired)

Stage 8 removed the prompt-debug persistence, SSE event, clipboard export, and
debug routes. The production daemon no longer writes prompt contents, prompt
metadata, hidden reasoning, or response previews to a debug file, and the
Swift client no longer requests or displays those records.

This file remains as a historical note for older installations. Its former
paths and examples are not supported runtime interfaces and must not be used
as release evidence.
