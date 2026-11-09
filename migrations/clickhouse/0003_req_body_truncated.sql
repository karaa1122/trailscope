-- Record separately whether the request body was cut, so replay can tell a
-- complete request that had a long (cut) response from a request it can't resend
-- faithfully. Rows written before this migration inherit body_truncated, which
-- may mean either body: the cautious answer.

ALTER TABLE {{db}}.requests
    ADD COLUMN IF NOT EXISTS req_body_truncated UInt8 DEFAULT body_truncated AFTER body_truncated;
