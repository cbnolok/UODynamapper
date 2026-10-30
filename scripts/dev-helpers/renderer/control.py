#!/usr/bin/env python3
"""Control an opt-in Unix Dynamapper preview without activating its window."""
import argparse
import json
from pathlib import Path
import socket
import tempfile
import time


def request(endpoint, command, timeout):
    with tempfile.TemporaryDirectory(prefix='dynamapper-ctl-') as directory:
        with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as client:
            client.bind(str(Path(directory) / 'reply.sock'))
            client.settimeout(timeout)
            client.sendto(json.dumps(command).encode(), str(endpoint))
            result = json.loads(client.recv(16384))
    if not result.get('ok'):
        raise RuntimeError(result.get('error', 'invalid control response'))
    return result


def png_complete(path):
    try:
        with path.open('rb') as stream:
            if stream.read(8) != b'\x89PNG\r\n\x1a\n':
                return False
            stream.seek(-12, 2)
            return stream.read(12) == b'\x00\x00\x00\x00IEND\xaeB`\x82'
    except (FileNotFoundError, OSError):
        return False


def wait_for_input(endpoint, timeout):
    deadline = time.monotonic() + timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('input did not finish')
        status = request(endpoint, {'command': 'status'}, remaining)
        if not status['input']['queued_frames']:
            return status
        time.sleep(0.01)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--socket', type=Path, required=True)
    parser.add_argument('--timeout', type=float, default=30)
    actions = parser.add_subparsers(dest='command', required=True)
    teleport = actions.add_parser('teleport')
    teleport.add_argument('x', type=int)
    teleport.add_argument('y', type=int)
    teleport.add_argument('--z', type=int, default=0)
    teleport.add_argument('--map', type=int, default=0)
    actions.add_parser('status')
    capture = actions.add_parser('capture')
    capture.add_argument('path', type=Path)
    key = actions.add_parser('key', help='Send a named Bevy KeyCode, for example F2 or ControlLeft')
    key.add_argument('key')
    key.add_argument('--state', choices=['tap', 'press', 'release'], default='tap')
    key.add_argument('--text', help='Optional logical text produced by this key')
    text = actions.add_parser('text', help='Type text into the current app input field')
    text.add_argument('text')
    pointer = actions.add_parser('pointer', help='Move the app-only pointer in logical window pixels')
    pointer.add_argument('x', type=float)
    pointer.add_argument('y', type=float)
    click = actions.add_parser('click')
    click.add_argument('x', type=float)
    click.add_argument('y', type=float)
    click.add_argument('--button', choices=['left', 'right', 'middle', 'back', 'forward'], default='left')
    click.add_argument('--state', choices=['tap', 'press', 'release'], default='tap')
    scroll = actions.add_parser('scroll', help='Scroll horizontal/vertical line deltas at the app pointer')
    scroll.add_argument('x', type=float)
    scroll.add_argument('y', type=float)
    actions.add_parser('reset-input', help='Discard pending input and release controller-held keys/buttons')
    args = parser.parse_args()
    command = {'command': args.command}
    if args.command == 'teleport':
        command.update(x=args.x, y=args.y, z=args.z, map=args.map)
    if args.command == 'capture':
        command['path'] = str(args.path.resolve())
    if args.command == 'key':
        command.update(key=args.key, state=args.state)
        if args.text is not None:
            command['text'] = args.text
    if args.command == 'text':
        command['text'] = args.text
    if args.command in ['pointer', 'click', 'scroll']:
        command.update(x=args.x, y=args.y)
    if args.command == 'click':
        command.update(button=args.button, state=args.state)
    if args.command == 'reset-input':
        command['command'] = 'reset_input'
    result = request(args.socket, command, args.timeout)
    if args.command in ['key', 'text', 'pointer', 'click', 'scroll', 'reset-input']:
        # Acknowledgement means queued. Drain press/release frames; UI animation,
        # settings saves and GPU uploads may still need settling before capture.
        wait_for_input(args.socket, args.timeout)
        result['input_queue_empty'] = True
    if args.command == 'capture':
        path = Path(command['path'])
        deadline = time.monotonic() + args.timeout
        while not png_complete(path):
            if time.monotonic() >= deadline:
                raise TimeoutError(f'capture did not finish: {path}')
            time.sleep(0.05)
        result['capture_complete'] = str(path)
    print(json.dumps(result))


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, RuntimeError, TimeoutError) as error:
        raise SystemExit(str(error)) from error
