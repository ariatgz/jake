-- Point three seeded companies at the boards that actually list their
-- internships, and drop the tracking junk that broke Marvell.
--
-- * Marvell's board URL had picked up a Google Analytics `_gl=` parameter and a
--   `Country` facet. Workday rejected the `_gl` facet with HTTP 400 on every run.
-- * Astera Labs keeps its interns and new-grad roles on a separate Greenhouse
--   board (`asteraearlycareer2027`), embedded in asteralabs.com/early-career-programs.
-- * Tenstorrent's university page lists the same Greenhouse board; only the
--   homepage link changes.
UPDATE companies
SET url = 'https://www.marvell.com/',
    board_type = 'WORKDAY',
    board_url = 'https://marvell.wd1.myworkdayjobs.com/MarvellCareers?workerSubType=65dea26481d001e09dfbab4927173419'
WHERE display_name = 'Marvell';

UPDATE companies
SET url = 'https://tenstorrent.com/en/careers/university',
    board_type = 'GREENHOUSE',
    board_url = 'https://job-boards.greenhouse.io/tenstorrent'
WHERE display_name = 'Tenstorrent';

UPDATE companies
SET url = 'https://www.asteralabs.com/early-career-programs/',
    board_type = 'GREENHOUSE',
    board_url = 'https://job-boards.greenhouse.io/asteraearlycareer2027'
WHERE display_name = 'Astera Labs';

-- onsemi moved from Workday to Oracle Cloud HCM; the old site answers HTTP 422.
ALTER TABLE companies DROP CONSTRAINT IF EXISTS company_board_type_check;
ALTER TABLE companies
ADD CONSTRAINT company_board_type_check
CHECK (board_type IN ('CUSTOM', 'WORKDAY', 'JIBE', 'GREENHOUSE', 'LEVER', 'ASHBY', 'ORACLE'));

UPDATE companies
SET url = 'https://www.onsemi.com/careers/students',
    board_type = 'ORACLE',
    board_url = 'https://hctz.fa.us2.oraclecloud.com/hcmUI/CandidateExperience/en/sites/CX_1001/requisitions?keyword=Intern'
WHERE display_name = 'onsemi';
