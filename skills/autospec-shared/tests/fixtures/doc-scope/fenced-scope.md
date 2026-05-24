# Fenced Code Block Handling

This document exercises fenced-code-block awareness in scan-doc-scope.

## Real Declaration

This is a real prose declaration and must be honored.

<!-- autospec-doc-scope:
  src: ["scripts/real.sh"]
  reason: "live prose declaration"
-->

Body for the real section.

## Illustrative Example

The following fenced block shows the syntax but must NOT be parsed as a live claim:

```markdown
<!-- autospec-doc-scope:
  src: ["scripts/example-in-fence.sh"]
  reason: "illustrative only"
-->
```

And a language-tagged fence, also ignored:

```bash
<!-- autospec-doc-scope:
  src: ["scripts/another-in-fence.sh"]
-->
```

A tilde fence, also ignored:

~~~
<!-- autospec-doc-scope:
  src: ["scripts/tilde-in-fence.sh"]
-->
~~~

## Post Fence Declaration

After all fences are closed, this prose declaration must be honored again.

<!-- autospec-doc-scope:
  src: ["scripts/post-fence.sh"]
-->

Body for the post-fence section.
