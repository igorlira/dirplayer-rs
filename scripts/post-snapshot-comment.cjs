// Post or update a snapshot test report as a PR comment.
// Called by actions/github-script in the e2e workflow.
// Accepts { github, context, label, artifactName } where label distinguishes
// native vs browser comments and artifactName is the artifact to link to.
module.exports = async ({ github, context, label, artifactName }) => {
  const fs = require("fs");

  const commentPath = "/tmp/diff-report/comment.md";
  const imageUrlPath = "/tmp/diff-report/image-url.txt";
  if (!fs.existsSync(commentPath)) return;

  let body = fs.readFileSync(commentPath, "utf8");

  // Prefix the title with the label
  body = body.replace(
    "### Snapshot Test Report",
    `### Snapshot Test Report (${label})`
  );

  const runUrl = `${context.serverUrl}/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}`;
  const artifact = artifactName || "test-results";

  // Embed the S3 image URL if the upload step produced one
  const imageUrl = fs.existsSync(imageUrlPath)
    ? fs.readFileSync(imageUrlPath, "utf8").trim()
    : null;

  if (imageUrl) {
    body = body.replace(
      "![Snapshot diffs](diff-grid.png)",
      `![Snapshot diffs](${imageUrl})`
    );
  } else {
    body = body.replace(
      "![Snapshot diffs](diff-grid.png)",
      `> Diff grid available in the [${artifact}](${runUrl}#artifacts) artifact.`
    );
  }

  body += `\n[View full run & download artifacts](${runUrl})\n`;

  // Find and update existing comment for this label, or create new one
  const marker = `<!-- snapshot-test-report:${label} -->`;
  body = marker + "\n" + body;
  const { data: comments } = await github.rest.issues.listComments({
    owner: context.repo.owner,
    repo: context.repo.repo,
    issue_number: context.issue.number,
  });
  const existing = comments.find((c) => c.body?.includes(marker));
  if (existing) {
    await github.rest.issues.updateComment({
      owner: context.repo.owner,
      repo: context.repo.repo,
      comment_id: existing.id,
      body,
    });
  } else {
    await github.rest.issues.createComment({
      owner: context.repo.owner,
      repo: context.repo.repo,
      issue_number: context.issue.number,
      body,
    });
  }
};
