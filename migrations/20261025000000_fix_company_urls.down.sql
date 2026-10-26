UPDATE companies
SET url = 'https://www.marvell.com/',
    board_type = 'WORKDAY',
    board_url = 'https://marvell.wd1.myworkdayjobs.com/MarvellCareers?workerSubType=65dea26481d001e09dfbab4927173419'
WHERE display_name = 'Marvell';

UPDATE companies
SET url = 'https://tenstorrent.com/',
    board_type = 'GREENHOUSE',
    board_url = 'https://job-boards.greenhouse.io/tenstorrent'
WHERE display_name = 'Tenstorrent';

-- Jobs scraped from the early-career board go, since the main board never listed them.
DELETE FROM applications
WHERE company_id IN (SELECT id FROM companies WHERE display_name = 'Astera Labs')
  AND url LIKE 'https://job-boards.greenhouse.io/asteraearlycareer2027/%';

UPDATE companies
SET url = 'https://www.asteralabs.com/',
    board_type = 'GREENHOUSE',
    board_url = 'https://job-boards.greenhouse.io/asteralabs'
WHERE display_name = 'Astera Labs';

DELETE FROM applications
WHERE company_id IN (SELECT id FROM companies WHERE display_name = 'onsemi');

UPDATE companies
SET url = 'https://www.onsemi.com/',
    board_type = 'WORKDAY',
    board_url = 'https://onsemi.wd1.myworkdayjobs.com/onsemi'
WHERE display_name = 'onsemi';

ALTER TABLE companies DROP CONSTRAINT IF EXISTS company_board_type_check;
ALTER TABLE companies
ADD CONSTRAINT company_board_type_check
CHECK (board_type IN ('CUSTOM', 'WORKDAY', 'JIBE', 'GREENHOUSE', 'LEVER', 'ASHBY'));
