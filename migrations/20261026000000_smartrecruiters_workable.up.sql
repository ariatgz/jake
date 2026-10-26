ALTER TABLE companies DROP CONSTRAINT IF EXISTS company_board_type_check;
ALTER TABLE companies
ADD CONSTRAINT company_board_type_check
CHECK (board_type IN ('CUSTOM', 'WORKDAY', 'JIBE', 'GREENHOUSE', 'LEVER', 'ASHBY', 'ORACLE', 'SMARTRECRUITERS', 'WORKABLE'));
