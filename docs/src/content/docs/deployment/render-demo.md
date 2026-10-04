---
title: Online demo on Render
description: Deploy Zhang without Docker on Render's free web service, backed by a read-only S3 or Cloudflare R2 bucket.
---

The repository's `render.yaml` builds the frontend into the Rust binary and runs it as a native Rust web service on Render's **Free** instance. The ledger lives in S3-compatible storage; no persistent disk or database is needed on Render.

## Prepare read-only storage

1. Use a dedicated bucket containing only public demo data. For a first demo, upload `examples/main.zhang` as `main.zhang`. Upload any included files, attachments and plugin modules with their relative paths preserved.
2. Give the service credentials **read and list** access to that bucket, with no write or delete access. Keep the credentials used to upload or maintain the sample separate from those used by Render.
   - **Cloudflare R2:** create an S3 API token with **Object Read only**, scoped to the demo bucket. Use its Access Key ID and Secret Access Key, the S3 endpoint `https://<account_id>.r2.cloudflarestorage.com`, and region `auto`. Use the jurisdiction-specific endpoint if your bucket requires one. The bucket can remain private; Zhang serves its contents to visitors. See [R2 authentication](https://developers.cloudflare.com/r2/api/tokens/).
   - **Amazon S3:** use a dedicated IAM identity with `s3:GetObject` on the demo objects and `s3:ListBucket` on the demo bucket. If the ledger is encrypted with a customer-managed KMS key, also allow decryption with that key. Use the bucket's actual region and its regional S3 endpoint. See [S3 policy examples](https://docs.aws.amazon.com/AmazonS3/latest/userguide/example-policies-s3.html).

Read-only credentials protect the source files even when a visitor sends a write request directly to the API. They do **not** hide editing controls in the web UI: saving a file, creating or updating a transaction, uploading a document or registering a passkey fails because storage refuses the write. Browsing, queries and CSV exports work normally. Leave password and passkey authentication disabled for an anonymous demo.

## Deploy the Blueprint

1. Push the deployment files to a GitHub branch. The demo can deploy from a pull request branch; merging is not required.
2. In the [Render Dashboard](https://dashboard.render.com), select **New → Blueprint**, connect the repository, and select that branch. Keep the Blueprint path `render.yaml`.
3. Fill in the variables Render prompts for:

   | Variable | Value |
   | --- | --- |
   | `ZHANG_S3_BUCKET` | The demo bucket's name. |
   | `ZHANG_S3_ENDPOINT` | The S3 API endpoint, not a public download URL. |
   | `ZHANG_S3_REGION` | `auto` for R2; the actual bucket region for AWS S3. |
   | `ZHANG_S3_ACCESS_KEY_ID` | The read-only Access Key ID. |
   | `ZHANG_S3_SECRET_ACCESS_KEY` | The read-only Secret Access Key. |

4. Review the service: **Rust**, **Free**, Singapore, and no disk or database. Deploy it.
5. If your ledger is not at the bucket root, set `ZHANG_S3_ROOT` to its prefix (for example `/accounting`). If its main file is not `main.zhang`, set `ZHANG_DEMO_ENDPOINT`. Save these changes before deploying, or redeploy afterwards. A `.bean` endpoint also works. For temporary AWS credentials, add `ZHANG_S3_SESSION_TOKEN`; renew the credentials before they expire.
6. Open the service's `onrender.com` URL and verify that the expected accounts and transactions appear. Also run an Explore query and export its CSV. To verify storage permissions, use the **demo credentials** to attempt writing a separate scratch object and confirm that storage rejects it.

The build script pins pnpm 9, builds the UI, and compiles only the `zhang` binary with the embedded-frontend feature. The start script validates the required settings and listens on `0.0.0.0:$PORT`. `/api/info` is the health check; inspect the accounts as well, because a missing main file can yield an empty ledger with a successful health check.

Automatic deployments are disabled to avoid spending build minutes on every commit. Use **Manual Deploy** after updating the branch. When you update the bucket's files, use Zhang's reload button or restart the service: Zhang does not watch remote storage. Source files remain in the bucket across Render redeploys and restarts; local document and plugin caches can be rebuilt.

## Free-tier limits

Render Free services sleep after 15 minutes without incoming traffic and take about a minute to wake. A workspace has 750 free instance hours per month, shared by its Free services. Free services also consume build minutes and outbound bandwidth. Without a payment method, exceeding the included quotas suspends services or disables builds instead of billing for additional usage. If the workspace already has a payment method, review its billing and spend settings separately: `plan: free` does not by itself disable every usage charge. See [Render's free-tier limits](https://render.com/docs/free).

R2 Standard storage has a monthly free allowance, including 10 GB-months of storage, 1 million Class A operations and 10 million Class B operations; direct R2 egress is free. Usage beyond the allowance is billed, so read-only access does not mean unlimited free storage or requests. See [R2 pricing](https://developers.cloudflare.com/r2/pricing/).
