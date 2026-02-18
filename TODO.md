# Interop TODO

## Done
- **ConPTY support**: Interactive mode uses Windows pseudo-console (ConPTY) when stdin is a TTY. Client enables raw mode and forwards VT sequences. Supports terminal resize via SIGWINCH.

## Future Ideas
- Signal forwarding (Ctrl+C as ConPTY input vs SIGINT)
- Environment variable passthrough from Linux to Windows
- Connection retry/reconnect on network drops
