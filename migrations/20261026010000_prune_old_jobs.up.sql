-- URLs of deleted old postings, so one still listed on its board isn't scraped and sent again.
CREATE TABLE pruned_urls (
  url TEXT PRIMARY KEY,
  company_id BIGINT NOT NULL,
  pruned_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_pruned_urls_company_id ON pruned_urls (company_id);
CREATE INDEX IF NOT EXISTS idx_applications_create_time ON applications (create_time);
