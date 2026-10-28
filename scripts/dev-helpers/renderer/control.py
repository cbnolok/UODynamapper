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
    args = parser.parse_args()
    command = {'command': args.command}
    if args.command == 'teleport':
        command.update(x=args.x, y=args.y, z=args.z, map=args.map)
    if args.command == 'capture':
        command['path'] = str(args.path.resolve())
    result = request(args.socket, command, args.timeout)
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
