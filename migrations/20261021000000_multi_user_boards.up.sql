-- Generic job boards -------------------------------------------------------
ALTER TABLE companies
ADD COLUMN board_type TEXT NOT NULL DEFAULT 'CUSTOM';

ALTER TABLE companies
ADD CONSTRAINT company_board_type_check
CHECK (board_type IN ('CUSTOM', 'WORKDAY', 'JIBE', 'GREENHOUSE', 'LEVER', 'ASHBY'));

-- Career board URL the scraper reads. Query params (Workday facets, Jibe filters) are kept.
ALTER TABLE companies
ADD COLUMN board_url TEXT NULL;

CREATE UNIQUE INDEX idx_companies_board_url ON companies (board_url) WHERE board_url IS NOT NULL;

-- Applications: wages are optional for generic boards, thread_id moves to notifications.
ALTER TABLE applications ALTER COLUMN lower_wage_cents DROP NOT NULL;
ALTER TABLE applications ALTER COLUMN thread_id DROP NOT NULL;
ALTER TABLE applications DROP CONSTRAINT applications_thread_id_key;
ALTER TABLE applications DROP CONSTRAINT application_concurrency_check;

-- Country codes are best-effort for scraped locations.
ALTER TABLE cities ALTER COLUMN country TYPE TEXT;

-- Per-user profiles ---------------------------------------------------------
CREATE TABLE profiles (
  user_id BIGINT PRIMARY KEY, -- Discord user id
  display_name TEXT NOT NULL,
  channel_id BIGINT NOT NULL, -- Discord channel that receives notifications
  keywords TEXT[] NOT NULL DEFAULT '{}', -- domain filter, empty = any
  seniority TEXT[] NOT NULL DEFAULT '{Intern,Internship,Co-op,Apprentice}', -- title must contain one
  locations TEXT[] NOT NULL DEFAULT '{}', -- cities/regions/countries, empty = any
  create_time TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  update_time TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE profile_companies (
  user_id BIGINT NOT NULL REFERENCES profiles(user_id) ON DELETE CASCADE,
  company_id BIGINT NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
  PRIMARY KEY (user_id, company_id)
);

-- One row per (job, user) that has been posted to Discord. This is the anti-spam record.
CREATE TABLE notifications (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  application_id BIGINT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
  user_id BIGINT NOT NULL,
  channel_id BIGINT NOT NULL,
  message_id BIGINT NOT NULL UNIQUE, -- also the thread id once a thread is opened from it
  state TEXT NOT NULL DEFAULT 'ACTIVE',
  create_time TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (application_id, user_id),
  CONSTRAINT notification_state_check CHECK (state IN ('ACTIVE', 'SUBMITTED', 'REJECTED', 'DELETED', 'IGNORED'))
);

CREATE INDEX idx_notifications_user_id ON notifications (user_id);

ALTER TABLE application_events ADD COLUMN user_id BIGINT NULL;

-- Carry over any legacy single-user postings so their buttons keep working.
INSERT INTO notifications (application_id, user_id, channel_id, message_id, state)
SELECT id, 0, 0, thread_id, state FROM applications WHERE thread_id IS NOT NULL;

-- Seed hardware companies -----------------------------------------------------
INSERT INTO companies (display_name, url, hex_code, icon_url, board_type, board_url) VALUES
  ('AMD', 'https://www.amd.com/', '#ED1C24',
   'https://www.google.com/s2/favicons?domain=amd.com&sz=128',
   'JIBE', 'https://careers.amd.com/careers-home/jobs?country=Canada&page=1'),
  ('Altera', 'https://www.altera.com/', '#0068B5',
   'https://www.google.com/s2/favicons?domain=altera.com&sz=128',
   'WORKDAY', 'https://altera.wd1.myworkdayjobs.com/Altera?jobFamilyGroup=3a3ac0bdef3a100fb57f7caeff190000'),
  ('Intel', 'https://www.intel.com/', '#0071C5',
   'https://www.google.com/s2/favicons?domain=intel.com&sz=128',
   'WORKDAY', 'https://intel.wd1.myworkdayjobs.com/External?jobFamilyGroup=dc8bf79476611087d67b36517cf17036'),
  ('NVIDIA', 'https://www.nvidia.com/', '#76B900',
   'https://www.google.com/s2/favicons?domain=nvidia.com&sz=128',
   'WORKDAY', 'https://nvidia.wd5.myworkdayjobs.com/NVIDIAExternalCareerSite?jobFamilyGroup=0c40f6bd1d8f10ae43ffda1e8d447e94'),
  ('onsemi', 'https://www.onsemi.com/', '#E65300',
   'https://www.google.com/s2/favicons?domain=onsemi.com&sz=128',
   'WORKDAY', 'https://onsemi.wd1.myworkdayjobs.com/onsemi'),
  ('Astera Labs', 'https://www.asteralabs.com/', '#5B3DF5',
   'https://www.google.com/s2/favicons?domain=asteralabs.com&sz=128',
   'GREENHOUSE', 'https://job-boards.greenhouse.io/asteralabs'),
  ('Etched', 'https://www.etched.com/', '#111111',
   'https://www.google.com/s2/favicons?domain=etched.com&sz=128',
   'ASHBY', 'https://jobs.ashbyhq.com/etched');
