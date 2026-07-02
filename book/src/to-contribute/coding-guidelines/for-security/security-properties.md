# Security Properties

### Validate at boundaries, trust internally (`validate-at-boundaries`) {#validate-at-boundaries}

Designate certain interfaces as validation boundaries.
In Asterinas, syscall entry points
are the primary boundary:
all user-supplied data
(pointers, file descriptors, sizes, flags, strings)
must be validated at the syscall boundary.
Once validated, internal kernel functions
may trust these values without re-validation.

See also:
PR [#2806](https://github.com/asterinas/asterinas/pull/2806).
