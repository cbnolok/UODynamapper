#!/usr/bin/env python3
"""Walk the player across the walking field of a running preview and check that nothing stops it.

For each direction and movement (walk, run), the player is teleported REACH cells before the field's centre and its
movement keys are held (W north, S south, A west, D east, two for a diagonal; Shift runs) until it is 2 * REACH cells
along the direction. Its position (status.position) is polled meanwhile: a pass fails when it stays on a cell for
longer than STALL_SECS (a collision with a static, an impassable tile or a step too high) or does not get there within
--max-secs. One JSON line per pass; the exit status is 1 when a pass failed.

    python3 -I -B walk_check.py --socket RUN/control.sock [--body 9] [--dirs N,E] [--modes walk]

The preview must already run with DYNAMAPPER_CONTROL_SOCKET; the style, sources and settings are left as they are.
--pause-secs holds still at the start of each pass, for a recorder started meanwhile (see SKILL.md).
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys
import time

CONTROL = Path(__file__).resolve().parents[3] / 'scripts/dev-helpers/renderer/control.py'
# The walking field: Trammel west of Yew, grass, dirt and a few small plants, z 0 along every ray (SKILL.md).
FIELD_CENTRE = (2111, 790, 0)
FIELD_MAP = 1
# Free cells along each of the 8 rays from the centre; a pass goes from -REACH to +REACH.
FIELD_REACH = 27
# Map step and held keys of each direction.
DIRECTIONS = {
    'N': ((0, -1), ['KeyW']), 'NE': ((1, -1), ['KeyW', 'KeyD']), 'E': ((1, 0), ['KeyD']),
    'SE': ((1, 1), ['KeyS', 'KeyD']), 'S': ((0, 1), ['KeyS']), 'SW': ((-1, 1), ['KeyS', 'KeyA']),
    'W': ((-1, 0), ['KeyA']), 'NW': ((-1, -1), ['KeyW', 'KeyA']),
}
RUN_KEY = 'ShiftLeft'
# A walking step takes 0.4 s, a running one 0.2 s (mounted: half); a cell held longer than this is a stall.
STALL_SECS = 1.0
POLL_SECS = 0.05
SCENE_TIMEOUT_SECS = 300.0
REQUEST_TIMEOUT_SECS = 40.0
# After the keys are released: the last step ends and the mobile settles.
SETTLE_SECS = 1.5


def load_control():
    spec = importlib.util.spec_from_file_location('dynamapper_control', CONTROL)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--socket', type=Path, required=True)
    parser.add_argument('--centre', default=','.join(map(str, FIELD_CENTRE)), help='x,y,z')
    parser.add_argument('--map', type=int, default=FIELD_MAP)
    parser.add_argument('--reach', type=int, default=FIELD_REACH)
    parser.add_argument('--dirs', default=','.join(DIRECTIONS))
    parser.add_argument('--modes', default='walk,run')
    parser.add_argument('--body', type=int, help='player_body to set first (9 the daemon, 132 the kirin)')
    parser.add_argument('--mount', type=int, help='player_mount_body to set first (0 none)')
    parser.add_argument('--pause-secs', type=float, default=0.0)
    parser.add_argument('--max-secs', type=float, default=40.0)
    args = parser.parse_args()
    control = load_control()

    def send(command, **fields):
        return control.request(args.socket, {'command': command, **fields}, REQUEST_TIMEOUT_SECS)

    def key(name, state):
        send('key', key=name, state=state)
        control.wait_for_input(args.socket, REQUEST_TIMEOUT_SECS)

    centre_x, centre_y, centre_z = (int(value) for value in args.centre.split(','))
    send('set', key='hide_player', value=False)
    if args.body is not None:
        send('set', key='player_body', value=args.body)
    if args.mount is not None:
        send('set', key='player_mount_body', value=args.mount)
    failed = False
    for mode in args.modes.split(','):
        for name in args.dirs.split(','):
            (dx, dy), keys = DIRECTIONS[name]
            start = (centre_x - dx * args.reach, centre_y - dy * args.reach)
            send('teleport', x=start[0], y=start[1], z=centre_z, map=args.map)
            control.wait_for_scene(args.socket, SCENE_TIMEOUT_SECS, [*start, centre_z, args.map])
            time.sleep(args.pause_secs)
            held = ([RUN_KEY] if mode == 'run' else []) + keys
            for held_key in held:
                key(held_key, 'press')
            began = time.monotonic()
            cell, since, cells, stalls, reached = None, began, 0, [], False
            while time.monotonic() - began < args.max_secs:
                position = send('status').get('position')
                now = time.monotonic()
                if position and tuple(position[:2]) != cell:
                    if cell is not None:
                        cells += 1
                        if now - since > STALL_SECS:
                            stalls.append([round(since - began, 2), round(now - began, 2), list(cell)])
                    cell, since = tuple(position[:2]), now
                # Progress along the direction, on a diagonal the axis behind: the first step of a diagonal can be
                # straight, its keys landing in different frames.
                if position and min(d * (p - s) for d, p, s in zip((dx, dy), position, start) if d) >= 2 * args.reach:
                    reached = True
                    break
                if now - since > STALL_SECS:
                    stalls.append([round(since - began, 2), None, list(cell) if cell else None])
                    break
                time.sleep(POLL_SECS)
            secs = round(time.monotonic() - began, 2)
            for held_key in reversed(held):
                key(held_key, 'release')
            send('reset_input')
            time.sleep(SETTLE_SECS)
            ok = reached and not stalls
            failed |= not ok
            print(json.dumps({'pass': f'{name}-{mode}', 'ok': ok, 'start': list(start),
                              'end': send('status').get('position'), 'cells': cells, 'secs': secs,
                              'stalls': stalls}), flush=True)
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
