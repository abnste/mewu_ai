# Linux worker lifetime admission

The factory is preparatory. Production constructors and the early application
bootstrap entry are not enabled by this packet. A raw Linux `Command` remains
unsupported before execution. Native prerequisite results do not establish
production factory acceptance.

`owned_process::linux_worker::TrustedCommand` accepts only six fixed host worker
variants. It opens the kernel's `/proc/self/exe`, retains that executable file,
and uses its descriptor for both bootstrap and domain execution. It does not
reopen `current_exe()` as a pathname. The fork hook marks descriptors close on
exec, then permits only the fixed source and bridge descriptors. Stdio remains
the original worker's configured stdio. The Rust spawn error pipe is preserved.

Installed bubblewrap creates a new user/PID namespace with the bootstrap as
PID 1. The bootstrap verifies its fixed arguments and loaded executable inode,
then sends its own kernel pidfd over the private bridge. The parent first owns
the original Child, provisional lifetime owner and operation permit in real
`Parts`; only after validating and retaining the receipt can it acknowledge.
The domain worker executes only after acknowledgement. Both extra descriptors
close during that final executable transition.

Construction, handshake and existing Ready share the caller's original earliest
absolute deadline and cancellation flag. Conservative conversions through
`CLOCK_MONOTONIC` can shorten that deadline, never create another startup budget.
For code recognition this means the original three-second Ready deadline, not
the longer total operation deadline.

Completion requires the actual namespace-init pidfd to signal completion,
original Child wait/reap and all tracked real IO readers to join. Killing the
outer wrapper or reaching a deadline is insufficient. Missing or malformed
receipts remain unproven even if the wrapper exited: the existing cleanup owner
retains the Child, witness/bridge, IO and operation permit. This can prevent
drain indefinitely and is an explicit availability limitation, not a successful
exit or update receipt.

The independent native fixture copies its own compiled executable into a
private directory, pins the loaded source, replaces its pathname and exposes
unrelated inheritable descriptors. It checks the actual worker's descriptor
whitelist, retained source identity, kernel PID1 completion, original wait and
permit retention until a deliberately blocked real IO reader joins. Typechecking
this fixture is preparation; it must run on native Linux before acceptance.

Additional native failure coverage is required for pre-exec cancellation and
expiry, cancellation during admission, absent/malformed ancillary receipts,
owner death before acknowledgement, bootstrap/final exec failure and bridge
EOF. Every case must distinguish confirmed drain from an unproven owner whose
permit remains retained. Ordinary command rejection, arbitrary MCP/daemon
programs, Windows and macOS lifetime implementations retain their boundaries.

References: [Rust current_exe security](https://doc.rust-lang.org/std/env/fn.current_exe.html),
[bubblewrap 0.6.1 source](https://github.com/containers/bubblewrap/blob/v0.6.1/bubblewrap.c),
[Linux dynamic-loader executable origin in glibc 2.35](https://github.com/bminor/glibc/blob/glibc-2.35/sysdeps/unix/sysv/linux/dl-origin.c).

The failure fixture uses a separate controller process for each case. Faults
exist only in that compiled fixture executable, not in production bootstrap
code. The test feature exposes Rust-only preparation/admission methods so a
real Child/job/permit can be retained without acknowledgement while testing
actual owner death before admission; no application IPC or command-line flag
exposes those methods. Default production builds omit these fixture methods.

Prepared cases cover pre-exec cancellation/expiry, marker-synchronized
cancellation after bootstrap entry but before a receipt, missing receipt,
incorrect tag or non-pidfd ancillary data, owner early exit, actual owner death
before acknowledgement, final executable-FD failure after acknowledgement and
bridge EOF. Unknown-owner cases explicitly assert UNPROVEN DRAIN and a retained
permit; their conservative failure assertion must not count as successful
application exit or update. All cases still require native execution.

The resource environment is derived from a verified AppDir instead of retaining
AppRun's inherited loader paths. The AppDir's `usr/bin/MewuAI` must match the
already-open loaded executable device/inode. Root identity and the bounded
resource tree are checked at capture and again immediately before spawning.
Normal internal library symlinks are allowed; escaping symlinks, special files,
cross-device trees and excessive depth/entries are rejected. Writable extracted
trees must have root/current-user ownership and no group/other writable entries.
Only an actual `fstatvfs` read-only filesystem permits writable stored mode bits.
Startup never changes resource permissions. This is identity/ownership validation,
not a cryptographic signature check or protection against the same user modifying
their own files during the dynamic loader's later reads.

Fixed GTK/GIO/GDKPixbuf/GStreamer paths are sealed after every caller environment
change. Arbitrary `LD_LIBRARY_PATH`, `LD_PRELOAD`, `LD_AUDIT`, loader tunables and
module search overrides are removed before exec, including standalone workers.
Missing required AppDir resources fail before creating a child. The actual
candidate's main ELF and 101 bundled reachable SONAMEs have relative RUNPATHs,
but 22 system SONAMEs, dynamically loaded modules and helper processes still need
native package checks. This static evidence does not establish worker Ready.

The added native resource fixtures run the real factory from a private AppDir
copy, test environment clearing/tainted overrides and normal internal SONAME
symlinks, and reject wrong/replaced loaded inodes, writable resources, escaping
symlinks and missing required files before exec. They are not an AppImage mount,
real GTK/GStreamer module load, signed installer, or application GUI acceptance.

The first actual factory run failed after owner admission because ACK validation
required empty returned flags. Linux returns the requested `MSG_CMSG_CLOEXEC`
in `msg_flags` even without ancillary data; rustix exposes that returned flag.
ACK validation accepts only this known echo, one byte and the exact ACK tag;
truncation and all other flags remain rejected. A native UnixStream fixture
records the real syscall echo before exercising the actual factory. The failed
run remains failure evidence until the corrected factory passes native CI.
See [Linux 6.8 recvmsg implementation](https://github.com/torvalds/linux/blob/v6.8/net/socket.c).

Runtime prerequisites are separate from the portable application's ELF bundle.
The factory uses installed `/usr/bin/bwrap`; this AppImage does not include that
helper. The host must permit the required user/PID namespaces and provide usable
`/proc`, pidfds, SCM_RIGHTS and `close_range(CLOSE_RANGE_CLOEXEC)`. The last flag
requires Linux 5.11 or later (including an equivalent backport); pidfd_open starts
at Linux 5.3. Kernel version alone does not establish namespace/syscall permission.
Missing helpers or denied syscalls remain unavailable and fail closed, rather
than falling back to an unowned raw process. No code changes OS policy, installs
helpers or weakens the existing deadline. This prerequisite set is not a claim
that portable startup, media backend Ready or application installation passed.
See [close_range](https://www.man7.org/linux/man-pages/man2/close_range.2.html) and
[pidfd_open](https://man7.org/linux/man-pages/man2/pidfd_open.2.html).
