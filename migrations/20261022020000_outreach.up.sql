-- People the user messaged about a job, and when to remind them to follow up.
CREATE TABLE outreach (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  notification_id BIGINT NOT NULL REFERENCES notifications(id) ON DELETE CASCADE,
  user_id BIGINT NOT NULL,
  contact_name TEXT NOT NULL,
  contact_url TEXT NULL,
  sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  follow_up_at TIMESTAMPTZ NULL, -- next reminder; NULL once no more are due
  follow_ups_sent SMALLINT NOT NULL DEFAULT 0,
  state TEXT NOT NULL DEFAULT 'WAITING',
  CONSTRAINT outreach_state_check CHECK (state IN ('WAITING', 'REPLIED', 'CLOSED'))
);

CREATE INDEX idx_outreach_due ON outreach (follow_up_at) WHERE state = 'WAITING';
CREATE INDEX idx_outreach_user ON outreach (user_id);

-- Search-engine results for recruiter lookups, so each query is paid for once
-- every couple of weeks. Only name, headline and profile URL are kept.
CREATE TABLE contact_search_cache (
  query TEXT PRIMARY KEY,
  results JSONB NOT NULL,
  fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
