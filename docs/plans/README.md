# Implementation plans

One file per task, `YYYY-MM-DD-<topic>.md`, moved to `done/` when the work
lands. Class B and C work is preceded by a plan; the template and the
frontmatter that carries owner claims are in
`docs/ENGINEERING_WORKFLOW.md` §2.4.

A plan is a thinking boundary, not a word count. A small fix gets a small
plan. Padding one to fill the template wastes the reviewer's attention and
hides the parts that matter — the failure states and the definition of done.

The frontmatter is also the concurrency protocol: `writes:` claims an owner
boundary for the life of the task, and another agent checks these files
before writing in the same owner (§4.1).
