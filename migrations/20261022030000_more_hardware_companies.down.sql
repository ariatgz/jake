-- applications.company_id has no foreign key, so remove the jobs first;
-- notifications and outreach rows go with them via ON DELETE CASCADE.
DELETE FROM applications
WHERE company_id IN (
  SELECT id FROM companies
  WHERE board_url IN (
    'https://marvell.wd1.myworkdayjobs.com/MarvellCareers?workerSubType=65dea26481d001e09dfbab4927173419',
    'https://cadence.wd1.myworkdayjobs.com/External_Careers?workerSubType=631f97c41c2749729ac5abe683f281d0',
    'https://ciena.wd5.myworkdayjobs.com/Careers?workerSubType=ac99acc6fb6b108fe66503995497ae7c',
    'https://analogdevices.wd1.myworkdayjobs.com/External?workerSubType=633b03df4f5d1000ec10ab4627a80000',
    'https://microchiphr.wd5.myworkdayjobs.com/External?zz_LRV_Career_Level_Requisition_Extended=57f4a35d19da0166ec5d6a6365019c99',
    'https://generalmotors.wd5.myworkdayjobs.com/Careers_GM?workerSubType=81219c91208501e94c213185a61a320a&workerSubType=81219c91208501ec00823185a61a360a',
    'https://bb.wd3.myworkdayjobs.com/BlackBerry',
    'https://job-boards.greenhouse.io/tenstorrent',
    'https://job-boards.greenhouse.io/lightmatter',
    'https://jobs.ashbyhq.com/cerebras',
    'https://jobs.lever.co/kepler'
  )
);

DELETE FROM companies
WHERE board_url IN (
  'https://marvell.wd1.myworkdayjobs.com/MarvellCareers?workerSubType=65dea26481d001e09dfbab4927173419',
  'https://cadence.wd1.myworkdayjobs.com/External_Careers?workerSubType=631f97c41c2749729ac5abe683f281d0',
  'https://ciena.wd5.myworkdayjobs.com/Careers?workerSubType=ac99acc6fb6b108fe66503995497ae7c',
  'https://analogdevices.wd1.myworkdayjobs.com/External?workerSubType=633b03df4f5d1000ec10ab4627a80000',
  'https://microchiphr.wd5.myworkdayjobs.com/External?zz_LRV_Career_Level_Requisition_Extended=57f4a35d19da0166ec5d6a6365019c99',
  'https://generalmotors.wd5.myworkdayjobs.com/Careers_GM?workerSubType=81219c91208501e94c213185a61a320a&workerSubType=81219c91208501ec00823185a61a360a',
  'https://bb.wd3.myworkdayjobs.com/BlackBerry',
  'https://job-boards.greenhouse.io/tenstorrent',
  'https://job-boards.greenhouse.io/lightmatter',
  'https://jobs.ashbyhq.com/cerebras',
  'https://jobs.lever.co/kepler'
);
