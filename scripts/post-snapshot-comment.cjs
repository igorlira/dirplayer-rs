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

  // Upload diff grid to S3 and embed in the comment
  if (fs.existsSync(diffGridPath)) {
    const imageUrl = await uploadToS3(diffGridPath, context, label);
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

// Upload a file to S3 and return its public URL.
// Requires AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, S3_BUCKET, and
// optionally S3_REGION (defaults to us-east-1) as environment variables.
async function uploadToS3(filePath, context, label) {
  const fs = require("fs");
  const crypto = require("crypto");

  const bucket = process.env.S3_BUCKET;
  const region = process.env.S3_REGION || "us-east-1";

  if (!process.env.AWS_ACCESS_KEY_ID || !process.env.AWS_SECRET_ACCESS_KEY || !bucket) {
    console.log("S3 credentials not configured, skipping image upload");
    return null;
  }

  try {
    const { S3Client, PutObjectCommand } = require("@aws-sdk/client-s3");

    const client = new S3Client({ region });
    const fileData = fs.readFileSync(filePath);
    const hash = crypto.createHash("sha256").update(fileData).digest("hex").slice(0, 12);
    const key = `dirplayer/snapshots/${context.runId}/${label}-${hash}.png`;

    await client.send(
      new PutObjectCommand({
        Bucket: bucket,
        Key: key,
        Body: fileData,
        ContentType: "image/png",
      })
    );

    const url = `https://${bucket}.s3.${region}.amazonaws.com/${key}`;
    console.log(`Diff grid uploaded to: ${url}`);
    return url;
  } catch (e) {
    console.log(`S3 upload error: ${e.message}`);
    return null;
  }
}
