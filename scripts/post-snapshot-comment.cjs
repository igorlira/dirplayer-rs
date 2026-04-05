// Post or update a snapshot test report as a PR comment.
// Called by actions/github-script in the e2e workflow.
// Accepts { github, context, label, artifactName } where label distinguishes
// native vs browser comments and artifactName is the artifact to link to.
module.exports = async ({ github, context, label, artifactName }) => {
  const fs = require("fs");

  const diffGridPath = "/tmp/diff-report/diff-grid.png";
  const commentPath = "/tmp/diff-report/comment.md";
  if (!fs.existsSync(commentPath)) return;

  let body = fs.readFileSync(commentPath, "utf8");

  // Prefix the title with the label
  body = body.replace(
    "### Snapshot Test Report",
    `### Snapshot Test Report (${label})`
  );

  const runUrl = `${context.serverUrl}/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}`;
  const artifact = artifactName || "test-results";

  // Upload diff grid image if it exists, and embed in the comment
  if (fs.existsSync(diffGridPath)) {
    const imageUrl = await uploadImage(github, context, diffGridPath);
    if (imageUrl) {
      body = body.replace(
        "![Snapshot diffs](diff-grid.png)",
        `![Snapshot diffs](${imageUrl})`
      );
    } else {
      body = body.replace(
        "![Snapshot diffs](diff-grid.png)",
        `> [Download diff grid & artifacts](${runUrl}#artifacts) from the **${artifact}** artifact.`
      );
    }
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

// Upload an image to GitHub via the repo's upload endpoint.
// Returns the public URL or null on failure.
async function uploadImage(github, context, filePath) {
  try {
    const fs = require("fs");
    const path = require("path");

    const fileName = path.basename(filePath);
    const fileData = fs.readFileSync(filePath);
    const fileSize = fileData.length;

    // Step 1: Request an upload policy
    const { data: policy } = await github.request(
      "POST /repos/{owner}/{repo}/uploads/policies/assets",
      {
        owner: context.repo.owner,
        repo: context.repo.repo,
        name: fileName,
        size: fileSize,
        content_type: "image/png",
      }
    );

    // Step 2: Upload the file using the policy
    const uploadUrl = policy.upload_url;
    const formData = new FormData();
    for (const [key, value] of Object.entries(policy.form || {})) {
      formData.append(key, value);
    }
    formData.append("file", new Blob([fileData], { type: "image/png" }), fileName);

    const uploadResponse = await fetch(uploadUrl, {
      method: "POST",
      body: formData,
    });

    if (!uploadResponse.ok) {
      console.log(`Image upload failed: ${uploadResponse.status}`);
      return null;
    }

    // The asset URL is in the policy response
    return policy.asset.href || policy.asset.original_url || null;
  } catch (e) {
    console.log(`Image upload error: ${e.message}`);
    return null;
  }
}
