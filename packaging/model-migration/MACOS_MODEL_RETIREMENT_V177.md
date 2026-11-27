# macOS model retirement candidate

This is the existing Windows signed retirement algorithm ported to macOS.
The optional signed payload field is `macosModelRetirementsV177`, with the
same four fields as the Windows directive. Windows directives do not authorize
macOS retirement. An omitted or empty array cancels eligibility.

The node requires a fresh signed registry, certified standby and replacement,
a healthy managed active replacement, seven days of continuous observation
(with no gap greater than 24 hours), an authenticated idle withdrawal grant,
and exact artifact hashes and ownership receipts. It refuses symlinks,
hardlinked files, custom model roots, shared artifacts, company/training
data and any artifact without an ownership receipt from a new verified download.
It deletes only the owned retired GGUF and its own receipt. Existing artifacts
without receipts are not adopted or deleted. The existing roster is refreshed
before the device returns to assignment eligibility.

This candidate does not publish a retirement directive, approve binary hashes,
change the backend, or authorize a release. The exact candidate identity must
be certified and allowed by the existing migration endpoint before activation.
