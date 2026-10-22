DELETE FROM companies WHERE board_type <> 'CUSTOM';

ALTER TABLE application_events DROP COLUMN user_id;
DROP TABLE notifications;
DROP TABLE profile_companies;
DROP TABLE profiles;

ALTER TABLE cities ALTER COLUMN country TYPE CHAR(2);

ALTER TABLE applications ADD CONSTRAINT application_concurrency_check CHECK (currency IN ('USD', 'CAD'));
ALTER TABLE applications ADD CONSTRAINT applications_thread_id_key UNIQUE (thread_id);
ALTER TABLE applications ALTER COLUMN thread_id SET NOT NULL;
ALTER TABLE applications ALTER COLUMN lower_wage_cents SET NOT NULL;

DROP INDEX idx_companies_board_url;
ALTER TABLE companies DROP COLUMN board_url;
ALTER TABLE companies DROP CONSTRAINT company_board_type_check;
ALTER TABLE companies DROP COLUMN board_type;
