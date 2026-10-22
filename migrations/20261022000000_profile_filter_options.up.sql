-- Title words that disqualify a posting (e.g. Senior, PhD, Sales).
ALTER TABLE profiles ADD COLUMN exclude_keywords TEXT[] NOT NULL DEFAULT '{}';

-- When true, domain keywords must appear in the job title, not just the description.
ALTER TABLE profiles ADD COLUMN keywords_title_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Highest application id already checked against this profile's filters, so each
-- run only looks at jobs stored since the last one. Reset to 0 whenever the
-- filters or tracked companies change; filters_version guards against a reset
-- that lands while a run is in progress.
ALTER TABLE profiles ADD COLUMN evaluated_through BIGINT NOT NULL DEFAULT 0;
ALTER TABLE profiles ADD COLUMN filters_version BIGINT NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_applications_company_id_id ON applications (company_id, id);
