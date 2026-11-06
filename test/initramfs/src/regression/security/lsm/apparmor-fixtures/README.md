# AppArmor policy fixtures

`observe.bin` is generated from `observe.profile` with AppArmor 5.0.2.
The other two binaries use the unmodified upstream inputs
`parser/tst/dirtest/gooddir/profileA` and `profileB` from
[AppArmor 5.0.2](https://gitlab.com/apparmor/apparmor/-/tree/909affc32a13d14ae3a7ccf44d325ae63ce11baf/parser/tst/dirtest/gooddir).
The upstream directory test uses these inputs to check parser directory handling.
Our kernel regression loads their compiled policies to check interoperability,
lexical query ordering, modes, and duplicate rejection.
This is not a run of the upstream kernel regression suite.

Regenerate with the pinned parser and extracted upstream source:

```sh
APPARMOR_SOURCE=/path/to/apparmor-v5.0.2 ./generate.sh
```

The script checks the upstream input hashes and freezes output sizes and hashes.
It disables compression and uses the local ABI v5 feature description.
`profileA` uses the default enforce mode;
`profileB` is compiled with `--Complain`.
The source inputs are not rewritten.

The upstream source release is licensed under GPL-2.0;
its archive includes `LICENSE` and the complete original test sources.
