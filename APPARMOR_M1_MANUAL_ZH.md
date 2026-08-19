# AppArmor M1 Guest 手工实验

本实验逐条观察 Asterinas 中已经落地的 AppArmor M1 数据面流程。每个实验命令只触发一个行为；不要把多个实验命令粘贴成一条命令执行。

当前 profile 是内核内置对象，不是用户态动态加载的文本 profile。当前没有真实 label 查询接口，因此输出中的 \`EXPECTED_PROFILE\` 是路径映射的预期值，\`PROFILE_QUERY=UNAVAILABLE\` 明确表示没有伪造内核实测 label。

## 0. 准备临时 Linux 副本

在 Windows PowerShell 中执行：

\`\`\`powershell
cd F:\App_Armor\asterinas
\`\`\`

启动 Asterinas 官方开发容器：

\`\`\`powershell
docker run --rm -it --privileged --network=host \`
  --name asterinas-apparmor-manual \`
  -v "F:\App_Armor\asterinas:/src:ro" \`
  asterinas/asterinas:0.17.1-20260319 bash
\`\`\`

进入容器后逐条执行：

\`\`\`bash
mkdir -p /work/asterinas
\`\`\`

\`\`\`bash
tar --exclude=target --exclude=.git -C /src -cf - . | tar -C /work/asterinas -xf -
\`\`\`

\`\`\`bash
cd /work/asterinas
\`\`\`

Windows 工作区中的部分 Nix/Make 文件可能是 CRLF。只在临时副本中转换构建相关文件：

\`\`\`bash
find . -type f \( -name '*.sh' -o -name '*.nix' -o -name 'Makefile' -o -name '*.mk' -o -name '*.toml' -o -path '*/test/initramfs/src/init' \) -exec sed -i 's/\r$//' {} +
\`\`\`

## 1. 构建 initramfs

逐条执行：

\`\`\`bash
make -C test/initramfs ENABLE_REGRESSION_TEST=true initramfs-boot-images
\`\`\`

\`\`\`bash
test -f test/initramfs/build/initramfs.cpio.gz
\`\`\`

预期：最后一条命令无输出并返回 0。

## 2. 构建 Asterinas 内核

\`\`\`bash
cargo osdk build --release
\`\`\`

## 3. 启动 Guest shell

\`\`\`bash
cargo osdk run --initramfs="$PWD/test/initramfs/build/initramfs.cpio.gz" -- /bin/sh
\`\`\`

启动完成后，在 Guest 中看到 shell 提示符，例如：

\`\`\`text
/ #
\`\`\`

之后的命令都在 Guest shell 中逐条执行。

## 4. 确认 AppArmor 已注册

执行：

\`\`\`sh
dmesg | grep 'LSM module enabled'
\`\`\`

预期包含：

\`\`\`text
[kernel] LSM module enabled: apparmor
\`\`\`

这一步对应内核启动时注册 LSM module，之后 capability/ptrace hook 才会进入 AppArmor 决策路径。

记录：

\`\`\`text
记录：________________________________________
\`\`\`

## 5. 实验一：unconfined baseline

执行：

\`\`\`sh
/test/security/lsm/apparmor-unconfined
\`\`\`

预期用户态输出：

\`\`\`text
STEP=unconfined-baseline
EXECUTABLE=/test/security/lsm/apparmor-unconfined
EXPECTED_PROFILE=kernel/default
PROFILE_QUERY=UNAVAILABLE
MODE=unconfined
OPERATION=chroot
AUDIT_CHECK=MANUAL_REQUIRED
RETURN=0
ERRNO=0
EXPECTED=RETURN=0,ERRNO=0
RESULT_SCOPE=USERSPACE_SYSCALL
RESULT=PASS
\`\`\`

预期审计：没有 AppArmor DENIED 记录。为避免使用旧日志作结论，先执行下面的统计并记录执行前数量：

\`\`\`sh
dmesg | grep apparmor | grep DENIED | wc -l
\`\`\`

执行入口后再次执行同一条统计命令；两次数量必须相同。

这一步对应：程序 exec 后没有匹配 M1 路径，任务保持 \`kernel/default\`，capability 请求被允许，\`chroot\` 成功。程序只验证用户态 syscall，审计需要手工前后比较。

记录：

\`\`\`text
用户态：____________________________________
审计：______________________________________
结论：______________________________________
\`\`\`

## 6. 实验二：Enforce 拒绝 CAP_SYS_CHROOT

执行：

\`\`\`sh
/test/security/lsm/apparmor-enforce-deny
\`\`\`

预期用户态输出：

\`\`\`text
STEP=enforce-capability-deny
EXECUTABLE=/test/security/lsm/apparmor-enforce-deny
EXPECTED_PROFILE=apparmor/m1-capability-test
PROFILE_QUERY=UNAVAILABLE
MODE=enforce
OPERATION=chroot
AUDIT_CHECK=MANUAL_REQUIRED
RETURN=-1
ERRNO=1
EXPECTED=RETURN=-1,ERRNO=1(EPERM)
RESULT_SCOPE=USERSPACE_SYSCALL
RESULT=PASS
\`\`\`

逐条查看最近审计：

\`\`\`sh
dmesg | grep apparmor | tail -n 1
\`\`\`

先记录执行入口前的 DENIED 数量，再执行入口，最后重新统计并比较数量；新增记录应包含 \`DENIED\`、\`capable\`、CAP_SYS_CHROOT 对应的 capability 18 和 \`enforce\`。

这一步对应：exec 路径匹配内置 Enforce profile，\`chroot\` 请求进入 capability LSM hook，AppArmor 返回 Deny，内核将其转换为用户态 \`EPERM\`。

记录：

\`\`\`text
用户态：____________________________________
审计：______________________________________
结论：______________________________________
\`\`\`

## 7. 实验三：Enforce 选择性放行 CAP_CHOWN

执行：

\`\`\`sh
/test/security/lsm/apparmor-enforce-allow
\`\`\`

预期用户态输出：

\`\`\`text
STEP=enforce-selective-allow
EXECUTABLE=/test/security/lsm/apparmor-enforce-allow
EXPECTED_PROFILE=apparmor/m1-capability-test
PROFILE_QUERY=UNAVAILABLE
MODE=enforce
OPERATION=fchown
RETURN=0
ERRNO=0
EXPECTED=RETURN=0,ERRNO=0
RESULT_SCOPE=USERSPACE_SYSCALL
RESULT=PASS
\`\`\`

预期审计：没有针对本次 \`fchown\` 的 AppArmor DENIED 记录。先执行 \`dmesg | grep apparmor | grep DENIED | wc -l\` 记录数量，执行入口后再次统计；两次数量必须相同，不能用旧的最后一条日志作结论。

这一步对应：同一个 Enforce profile 只禁止 capability 18；\`fchown\` 所需的 CAP_CHOWN 不在禁止列表中，因此允许通过。

记录：

\`\`\`text
用户态：____________________________________
审计：______________________________________
结论：______________________________________
\`\`\`

## 8. 实验四：fork 继承 profile

执行：

\`\`\`sh
/test/security/lsm/apparmor-fork
\`\`\`

预期用户态输出：

\`\`\`text
STEP=fork-inheritance
EXECUTABLE=/test/security/lsm/apparmor-fork
EXPECTED_PROFILE=apparmor/m1-capability-test
PROFILE_QUERY=UNAVAILABLE
MODE=enforce
OPERATION=fork->chroot
AUDIT_CHECK=MANUAL_REQUIRED
RETURN=-1
ERRNO=1
EXPECTED=RETURN=-1,ERRNO=1(EPERM)
RESULT_SCOPE=USERSPACE_SYSCALL
RESULT=PASS
\`\`\`

预期审计：包含子进程执行 \`chroot\` 时的 AppArmor DENIED。应在执行前后比较 DENIED 数量，并检查新增记录。

这一步对应：父进程 exec 后获得 Enforce profile，fork 创建子进程时复制 task label，子进程的 capability 请求仍被同一 profile 拒绝。

记录：

\`\`\`text
用户态：____________________________________
审计：______________________________________
结论：______________________________________
\`\`\`

## 9. 实验五：Complain 允许但记录

执行：

\`\`\`sh
/test/security/lsm/apparmor-complain
\`\`\`

预期用户态输出：

\`\`\`text
STEP=complain-capability-report
EXECUTABLE=/test/security/lsm/apparmor-complain
EXPECTED_PROFILE=apparmor/m1-complain-test
PROFILE_QUERY=UNAVAILABLE
MODE=complain
OPERATION=chroot
AUDIT_CHECK=MANUAL_REQUIRED
AUDIT_EXPECTED=complain/would-deny
RETURN=0
ERRNO=0
EXPECTED=RETURN=0,ERRNO=0
RESULT_SCOPE=USERSPACE_SYSCALL
RESULT=PASS
\`\`\`

随后查看审计。程序本身不能读取或验证内核审计，所以 \`RESULT=PASS\` 只表示用户态 \`chroot\` 成功：

\`\`\`sh
dmesg | grep apparmor | tail -n 1
\`\`\`

预期包含 \`complain\` 或 \`would-deny\`，但本次请求不应被转换为 \`EPERM\`。

这一步对应：profile 的策略判断仍发现 capability 18 不允许，但 Complain 模式把 Deny 转换为允许，同时保留审计信息。

记录：

\`\`\`text
用户态：____________________________________
审计：______________________________________
结论：______________________________________
\`\`\`

## 10. 实验六：跨 profile ptrace 拒绝

执行：

\`\`\`sh
/test/security/lsm/apparmor-ptrace
\`\`\`

该入口的父进程保持 \`kernel/default\`；子进程 exec \`/test/security/lsm/apparmor-enforce-deny\`，因此目标进程预期获得 \`apparmor/m1-capability-test\`。父进程只执行一次 \`PTRACE_ATTACH\`。

预期用户态输出：

\`\`\`text
STEP=cross-profile-ptrace-deny
EXECUTABLE=/test/security/lsm/apparmor-ptrace
EXPECTED_PROFILE=kernel/default
PROFILE_QUERY=UNAVAILABLE
MODE=enforce
OPERATION=ptrace(PTRACE_ATTACH)
AUDIT_CHECK=MANUAL_REQUIRED
RETURN=-1
ERRNO=1
EXPECTED=RETURN=-1,ERRNO=1(EPERM)
RESULT_SCOPE=USERSPACE_SYSCALL
RESULT=PASS
TRACER_EXPECTED_PROFILE=kernel/default
TARGET_EXPECTED_PROFILE=apparmor/m1-capability-test
\`\`\`

预期审计：手工检查新增 ptrace 拒绝记录；\`RESULT=PASS\` 只表示用户态 \`PTRACE_ATTACH\` 返回 \`EPERM\`，程序不读取内核审计。

这一步对应：ptrace hook 同时检查 tracer 和 target 的 task label；\`kernel/default\` 与 \`apparmor/m1-capability-test\` 不同，Enforce 决策拒绝 attach。

记录：

\`\`\`text
用户态：____________________________________
审计：______________________________________
结论：______________________________________
\`\`\`

## 11. 结束 Guest

实验结束后执行：

\`\`\`sh
poweroff -f
\`\`\`

## 路径到 profile 映射

| Guest 可执行路径 | 预期 profile | 模式 | 行为 |
| --- | --- | --- | --- |
| \`/test/security/lsm/apparmor-unconfined\` | \`kernel/default\` | unconfined | \`chroot\` 允许 |
| \`/test/security/lsm/apparmor-enforce-deny\` | \`apparmor/m1-capability-test\` | enforce | CAP_SYS_CHROOT 拒绝 |
| \`/test/security/lsm/apparmor-enforce-allow\` | \`apparmor/m1-capability-test\` | enforce | \`fchown\` 允许 |
| \`/test/security/lsm/apparmor-fork\` | \`apparmor/m1-capability-test\` | enforce | 子进程继承后拒绝 \`chroot\` |
| \`/test/security/lsm/apparmor-complain\` | \`apparmor/m1-complain-test\` | complain | \`chroot\` 允许并审计 |
| \`/test/security/lsm/apparmor-ptrace\` | \`kernel/default\` | enforce | attach 到 Enforce target 拒绝 |

原有 \`/test/security/lsm/apparmor\` 无参数综合回归保持不变。
