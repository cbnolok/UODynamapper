# Local renderer test control

On Unix, `DYNAMAPPER_CONTROL_SOCKET` opts a process into a private Unix datagram
control endpoint. Normal launches install no socket or polling system. This is
a development transport; Windows builds omit it. Commands are JSON datagrams,
limited to 4096 bytes and eight requests per frame, handled nonblockingly on the
main ECS thread in `MovementActions`.

`teleport`, `status` and `capture` are the only commands. There is no shell/code
execution or generic settings mutation. The owner-only socket refuses occupied
paths; normal resource cleanup only removes the bound inode. Forced termination
may leave a socket, so isolate each run in its own temporary directory.

The UI dialog and endpoint share `controls::teleport::teleport_player` for loaded
map/bounds validation and player/transform updates. Both request visible-chunk
recomputation after success, including same-map teleports. Invalid destinations
leave state untouched. This does not add loading for previously unloaded maps.

`status` exposes current and previously rendered player positions, elapsed time,
and the existing smoothed FPS/frame-time diagnostics when available. Rendered
position equality does not establish completion of texture streaming. Keep
settling time and multiple-frame inspection in the validation workflow.

`capture` uses Bevy's native screenshot path, so hidden/off-desktop windows do
not need activation. Existing files/symlinks, relative/non-PNG paths and missing
parent directories are rejected. The reply acknowledges queuing; the Python
client waits for the PNG signature and final IEND before declaring completion.

Reusable commands and transport documentation live in
`scripts/dev-helpers/renderer/`. They can drive one preview through multiple
locations without changing startup session state or rebuilding capture hooks.
