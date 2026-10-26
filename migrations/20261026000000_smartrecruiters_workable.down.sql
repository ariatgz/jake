-- applications.company_id has no foreign key, so remove the jobs first;
-- notifications and outreach rows go with them via ON DELETE CASCADE.
DELETE FROM applications
WHERE company_id IN (SELECT id FROM companies WHERE board_type IN ('SMARTRECRUITERS', 'WORKABLE'));

DELETE FROM companies WHERE board_type IN ('SMARTRECRUITERS', 'WORKABLE');

ALTER TABLE companies DROP CONSTRAINT IF EXISTS company_board_type_check;
ALTER TABLE companies
ADD CONSTRAINT company_board_type_check
CHECK (board_type IN ('CUSTOM', 'WORKDAY', 'JIBE', 'GREENHOUSE', 'LEVER', 'ASHBY', 'ORACLE'));
