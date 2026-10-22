DROP INDEX IF EXISTS idx_applications_company_id_id;

ALTER TABLE profiles DROP COLUMN filters_version;
ALTER TABLE profiles DROP COLUMN evaluated_through;
ALTER TABLE profiles DROP COLUMN keywords_title_only;
ALTER TABLE profiles DROP COLUMN exclude_keywords;
