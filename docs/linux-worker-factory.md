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
[Linux dynamic-loader executable origin](https://github.com/bminor/glibc/blob/master/sysdeps/unix/sysv/linux/dl-origin.c).

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
