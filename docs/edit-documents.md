# Edit documents

The prepare-edits command accepts one compact UTF-8 document for an ordered, guarded transaction across workspace files. Preparation never writes source files; the resulting plan is reviewed and applied through the existing plan workflow.

    threadmoth prepare-edits changes.tm --output prepared.json --summary
    threadmoth explain --plan prepared.json --format diff
    threadmoth apply-plan --plan prepared.json --summary

A document uses one file section per workspace-relative target. Put edits for the same file in execution order within its section:

    *** File: src/settings.txt
    <<<<<<< SEARCH
    mode=old
    =======
    mode=new
    >>>>>>> REPLACE

Each SEARCH must match exactly once at that point in the evolving source. A missing, duplicate, overlapping, out-of-workspace, or stale target refuses the plan without partial source changes. Later SEARCH text may match the result of an earlier block.

Delimiter lines are syntax. If a block body needs a line that is exactly one of the three delimiters, prefix that literal line with one backslash; the parser removes that escape. The delimiter's own terminating newline is syntax. Add an extra blank line immediately before a delimiter when the intended body itself ends in a newline.

The document may use LF or CRLF. Threadmoth normalizes the document framing, then adapts replacement text to a uniformly LF or CRLF source through the existing source-aware exact text provider. Mixed-newline source files are refused because a uniform source policy cannot preserve their line-ending pattern. Use the existing exact text preparation route when an operation needs explicit mixed-ending bytes.

The command accepts at most 64 file sections, 64 exact edits in each file section, 256 exact edits across the whole document, 1 MiB of input, and 8 MiB per serialized plan. The 65th edit in one section is refused with a line-numbered diagnostic. Review the complete diff before applying. Applying rechecks source hashes, exact file-specific authority, budgets, and the prepared transaction; success returns the standard transaction certificate and rollback guarantee.
