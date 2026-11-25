# SPDX-License-Identifier: MPL-2.0

"""Local lifecycle orchestration using the unchanged upstream introspect helper."""

import errno
import json
import os
from pathlib import Path
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import threading


HELPER = Path('/opt/apparmor/introspect')


def run(*args):
    subprocess.run([str(arg) for arg in args], check=True, timeout=30, stdout=sys.stderr)


def query(executable, target, name, mode=None):
    args = [executable, str(target), name]
    if mode is not None:
        args.append(mode)
    run(*args)


def ready(process):
    if not select.select([process.stdout], [], [], 30)[0]:
        raise RuntimeError('worker readiness timed out')
    return json.loads(process.stdout.readline())


def worker(root):
    stopped = threading.Event()
    started = threading.Event()
    tids = []

    def thread_body():
        tids.append(threading.get_native_id())
        started.set()
        stopped.wait()

    thread = threading.Thread(target=thread_body)
    thread.start()
    try:
        assert started.wait(30)
        print(json.dumps({'pid': os.getpid(), 'tid': tids[0]}), flush=True)
        for command in sys.stdin:
            if command.strip() == 'verify':
                query(HELPER, 'self', 'm2_label_python', 'enforce')
                child = os.fork()
                if child == 0:
                    try:
                        os.dup2(2, 1)
                        os.execv(HELPER, [str(HELPER), 'self', 'm2_label_python', 'enforce'])
                    finally:
                        os._exit(1)
                assert os.waitpid(child, 0)[1] == 0
                spawned = os.posix_spawn(
                    HELPER, [str(HELPER), 'self', 'm2_label_python', 'enforce'], os.environ,
                    file_actions=[(os.POSIX_SPAWN_DUP2, 2, 1)],
                )
                assert os.waitpid(spawned, 0)[1] == 0
                query(root / 'beta', 'self', 'm2_label_python', 'enforce')
                print('VERIFIED', flush=True)
            elif command.strip() == 'stop':
                break
    finally:
        stopped.set()
        thread.join(30)
        assert not thread.is_alive()


def main():
    root = Path(tempfile.mkdtemp(prefix='apparmor-m2-'))
    profiles = {}
    processes = []
    mounts = []

    def load(name, attachment, mode='enforce'):
        path = root / (name + '.profile')
        flags = ' flags=(complain)' if mode == 'complain' else ''
        path.write_text(f'profile {name} "{attachment}"{flags} {{\n}}\n')
        run('apparmor_parser', '--config-file=/etc/apparmor/parser.conf', '-q', '-K', path)
        profiles[name] = path

    def remove(name):
        run('apparmor_parser', '--config-file=/etc/apparmor/parser.conf', '-j0', '-K', '-R', profiles[name])
        del profiles[name]

    def copy_helper(name):
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(HELPER, path)
        return path

    def start_worker():
        process = subprocess.Popen(
            [sys.executable, __file__, '--worker', str(root)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        )
        processes.append(process)
        info = ready(process)
        query(HELPER, info['pid'], 'm2_label_python', 'enforce')
        query(HELPER, info['tid'], 'm2_label_python', 'enforce')
        return process, info

    try:
        exact = copy_helper('exact')
        load('m2_label_exact', exact)
        query(exact, 'self', 'm2_label_exact', 'enforce')
        beta = copy_helper('beta')
        load('m2_label_beta', beta, 'complain')
        query(beta, 'self', 'm2_label_beta', 'complain')
        query(HELPER, 'self', 'unconfined')
        # Match the upstream introspect.sh negative mode assertion.
        wrong_mode = subprocess.run(
            [HELPER, 'self', 'unconfined', 'enforce'],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30,
        )
        assert wrong_mode.returncode != 0
        link = root / 'link'
        link.symlink_to(exact)
        query(link, 'self', 'm2_label_exact', 'enforce')

        glob = copy_helper('glob1')
        load('m2_label_glob', root / 'glob*')
        query(glob, 'self', 'm2_label_glob', 'enforce')
        query(copy_helper('unmatched'), 'self', 'unconfined')
        overlap = copy_helper('overlap/suffix_long')
        load('m2_label_suffix', root / 'o*/suffix_long')
        load('m2_label_prefix', root / 'overlap/*')
        query(overlap, 'self', 'm2_label_prefix', 'enforce')
        remove('m2_label_suffix')
        remove('m2_label_prefix')
        load('m2_label_prefix', root / 'overlap/*')
        load('m2_label_suffix', root / 'o*/suffix_long')
        query(overlap, 'self', 'm2_label_prefix', 'enforce')
        remove('m2_label_suffix')
        load('m2_label_suffix', root / 'o{v,verlap}/suffix_long')
        query(overlap, 'self', 'm2_label_suffix', 'enforce')
        load('m2_label_ambiguous', root / 'o{v,verlap}/suffix_long')
        query(overlap, 'self', 'unconfined')
        remove('m2_label_ambiguous')
        remove('m2_label_prefix')
        remove('m2_label_suffix')

        # Linux chooses the longer actual left match when xmatch lengths tie.
        load('m2_label_first', root / 'o{v,verlap}/suffix_long')
        load('m2_label_second', root / 'overlap/suffix*')
        query(overlap, 'self', 'm2_label_first', 'enforce')
        remove('m2_label_first')
        remove('m2_label_second')

        # File policies are saved without attaching to tasks.
        file_policy = copy_helper('file-policy')
        name = 'm2_label_file_rules'
        profiles_file = Path('/sys/kernel/security/apparmor/profiles')
        before = profiles_file.read_text()
        for rule in ('deny /tmp/secret r,', '/tmp/secret r,', '/bin/true ux,'):
            source = f'profile {name} "{file_policy}" {{ {rule} }}\n'
            blob = subprocess.run(
                ['apparmor_parser', '--config-file=/dev/null', '-m',
                 'policy {versions {v5 {yes}}}', '-j0', '-K', '-S'],
                input=source.encode(), stdout=subprocess.PIPE, check=True, timeout=30,
            ).stdout
            with open('/sys/kernel/security/apparmor/.load', 'wb', buffering=0) as load_fd:
                assert load_fd.write(blob) == len(blob)
            try:
                assert f'{name} (enforce)' in profiles_file.read_text().splitlines()
                query(file_policy, 'self', 'unconfined')
            finally:
                with open('/sys/kernel/security/apparmor/.remove', 'wb', buffering=0) as remove_fd:
                    assert remove_fd.write(name.encode()) == len(name)
            assert profiles_file.read_text() == before

        script = root / 'script'
        script.write_text(
            f'#!{sys.executable}\nimport os\n'
            f'os.execv({str(HELPER)!r}, [{str(HELPER)!r}, "self", "m2_label_script", "enforce"])\n'
        )
        script.chmod(0o755)
        load('m2_label_script', script)
        run(script)

        jailed = copy_helper('jail/helper')
        load('m2_label_chroot', jailed)
        for source in ('/nix', '/proc'):
            target = root / 'jail' / source[1:]
            target.mkdir()
            run('mount', '--bind', source, target)
            mounts.append(target)
        child = os.fork()
        if child == 0:
            try:
                os.chroot(root / 'jail')
                os.chdir('/')
                os.execv('/helper', ['/helper', 'self', 'm2_label_chroot', 'enforce'])
            finally:
                os._exit(1)
        assert os.waitpid(child, 0)[1] == 0
        for target in reversed(mounts):
            run('umount', target)
        mounts.clear()

        bad = copy_helper('bad-elf')
        data = bytearray(bad.read_bytes())
        assert data[:6] == b'\x7fELF\x02\x01'
        offset = struct.unpack_from('<Q', data, 32)[0]
        size, count = struct.unpack_from('<HH', data, 54)
        for index in range(count):
            position = offset + index * size
            if struct.unpack_from('<I', data, position)[0] == 3:
                struct.pack_into('<I', data, position, 0)
        struct.pack_into('<Q', data, 24, (1 << 64) - 1)
        bad.write_bytes(data)
        load('m2_label_bad_exec', bad)
        report_read, report_write = os.pipe()
        release_read, release_write = os.pipe()
        child = os.fork()
        if child == 0:
            try:
                os.close(report_read)
                os.close(release_write)
                try:
                    os.execv(bad, [str(bad)])
                except OSError as error:
                    os.write(report_write, str(error.errno).encode())
                    os.close(report_write)
                    os.read(release_read, 1)
                    os._exit(0)
            finally:
                os._exit(1)
        os.close(report_write)
        os.close(release_read)
        try:
            if not select.select([report_read], [], [], 30)[0]:
                os.kill(child, signal.SIGKILL)
                raise TimeoutError('failed exec did not return')
            assert int(os.read(report_read, 32)) == errno.ENOEXEC
            query(HELPER, child, 'unconfined')
        finally:
            try:
                os.write(release_write, b'x')
            except BrokenPipeError:
                pass
            os.close(release_write)
            os.close(report_read)
            assert os.waitpid(child, 0)[1] == 0

        load('m2_label_python', os.path.realpath(sys.executable))
        first, first_info = start_worker()
        first.stdin.write('verify\n')
        first.stdin.flush()
        while True:
            assert select.select([first.stdout], [], [], 30)[0]
            line = first.stdout.readline()
            assert line, 'worker exited before verification'
            print(line, end='', flush=True)
            if line.strip() == 'VERIFIED':
                break

        shell = root / 'shell'
        shutil.copy2(shutil.which('bash'), shell)
        load('m2_label_shell', shell, 'complain')
        second = subprocess.Popen(
            [str(shell), '--noprofile', '--norc', '-c', 'echo READY; read -r done'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        )
        processes.append(second)
        assert select.select([second.stdout], [], [], 30)[0]
        assert second.stdout.readline().strip() == 'READY'
        query(HELPER, second.pid, 'm2_label_shell', 'complain')
        query(HELPER, first_info['pid'], 'm2_label_python', 'enforce')
        remove('m2_label_python')
        query(HELPER, first_info['pid'], 'unconfined')
        query(HELPER, first_info['tid'], 'unconfined')
        load('m2_label_python', os.path.realpath(sys.executable))
        query(HELPER, first_info['pid'], 'unconfined')
        third, _ = start_worker()
        for process in (first, third):
            process.stdin.write('stop\n')
            process.stdin.flush()
        second.stdin.write('done\n')
        second.stdin.flush()
        for process in processes:
            assert process.wait(timeout=30) == 0
        print('APPARMOR_UPSTREAM_LIFECYCLE_OK', flush=True)
    finally:
        cleanup_errors = []
        for process in processes:
            try:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=30)
            except Exception as error:
                cleanup_errors.append(error)
        for name in list(profiles):
            try:
                remove(name)
            except Exception as error:
                cleanup_errors.append(error)
        for target in reversed(list(mounts)):
            try:
                run('umount', target)
                mounts.remove(target)
            except Exception as error:
                cleanup_errors.append(error)
        # Keep a still-mounted directory intact so cleanup never traverses /nix.
        if not mounts:
            try:
                shutil.rmtree(root)
            except Exception as error:
                cleanup_errors.append(error)
        if cleanup_errors:
            raise ExceptionGroup('lifecycle cleanup failed', cleanup_errors)


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--worker':
        worker(Path(sys.argv[2]))
    else:
        main()
