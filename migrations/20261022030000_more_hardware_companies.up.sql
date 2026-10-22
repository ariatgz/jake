-- More hardware/firmware employers with Canadian offices. Workday boards are
-- limited to their intern/co-op category so the bot doesn't download every
-- full-time posting. URLs are in the canonical form `/add-company` produces.
INSERT INTO companies (display_name, url, hex_code, icon_url, board_type, board_url)
SELECT v.display_name, v.url, v.hex_code, v.icon_url, v.board_type, v.board_url
FROM (VALUES
  ('Marvell', 'https://www.marvell.com/', '#D52B1E',
   'https://www.google.com/s2/favicons?domain=marvell.com&sz=128',
   'WORKDAY', 'https://marvell.wd1.myworkdayjobs.com/MarvellCareers?workerSubType=65dea26481d001e09dfbab4927173419'),
  ('Cadence', 'https://www.cadence.com/', '#E4002B',
   'https://www.google.com/s2/favicons?domain=cadence.com&sz=128',
   'WORKDAY', 'https://cadence.wd1.myworkdayjobs.com/External_Careers?workerSubType=631f97c41c2749729ac5abe683f281d0'),
  ('Ciena', 'https://www.ciena.com/', '#E31837',
   'https://www.google.com/s2/favicons?domain=ciena.com&sz=128',
   'WORKDAY', 'https://ciena.wd5.myworkdayjobs.com/Careers?workerSubType=ac99acc6fb6b108fe66503995497ae7c'),
  ('Analog Devices', 'https://www.analog.com/', '#0067B9',
   'https://www.google.com/s2/favicons?domain=analog.com&sz=128',
   'WORKDAY', 'https://analogdevices.wd1.myworkdayjobs.com/External?workerSubType=633b03df4f5d1000ec10ab4627a80000'),
  ('Microchip', 'https://www.microchip.com/', '#EE2223',
   'https://www.google.com/s2/favicons?domain=microchip.com&sz=128',
   'WORKDAY', 'https://microchiphr.wd5.myworkdayjobs.com/External?zz_LRV_Career_Level_Requisition_Extended=57f4a35d19da0166ec5d6a6365019c99'),
  ('General Motors', 'https://www.gm.com/', '#0170CE',
   'https://www.google.com/s2/favicons?domain=gm.com&sz=128',
   'WORKDAY', 'https://generalmotors.wd5.myworkdayjobs.com/Careers_GM?workerSubType=81219c91208501e94c213185a61a320a&workerSubType=81219c91208501ec00823185a61a360a'),
  ('BlackBerry', 'https://www.blackberry.com/', '#1A1A1A',
   'https://www.google.com/s2/favicons?domain=blackberry.com&sz=128',
   'WORKDAY', 'https://bb.wd3.myworkdayjobs.com/BlackBerry'),
  ('Tenstorrent', 'https://tenstorrent.com/', '#7B5CFA',
   'https://www.google.com/s2/favicons?domain=tenstorrent.com&sz=128',
   'GREENHOUSE', 'https://job-boards.greenhouse.io/tenstorrent'),
  ('Lightmatter', 'https://lightmatter.co/', '#2B2B2B',
   'https://www.google.com/s2/favicons?domain=lightmatter.co&sz=128',
   'GREENHOUSE', 'https://job-boards.greenhouse.io/lightmatter'),
  ('Cerebras', 'https://www.cerebras.ai/', '#F15A29',
   'https://www.google.com/s2/favicons?domain=cerebras.ai&sz=128',
   'ASHBY', 'https://jobs.ashbyhq.com/cerebras'),
  ('Kepler Communications', 'https://kepler.space/', '#1C2B4A',
   'https://www.google.com/s2/favicons?domain=kepler.space&sz=128',
   'LEVER', 'https://jobs.lever.co/kepler')
) AS v (display_name, url, hex_code, icon_url, board_type, board_url)
-- Skip anything already added by hand with /add-company.
WHERE NOT EXISTS (
  SELECT 1 FROM companies AS c
  WHERE c.url = v.url OR c.board_url = v.board_url OR LOWER(c.display_name) = LOWER(v.display_name)
);

-- Profiles set up with the hardware_canada preset (tracking most of the
-- original chip companies) start tracking the new ones too. Their evaluation
-- watermark is reset so jobs already stored for these companies are considered.
WITH preset_users AS (
  SELECT pc.user_id
  FROM profile_companies AS pc
  JOIN companies AS c ON c.id = pc.company_id
  WHERE c.display_name IN ('AMD', 'Altera', 'Intel', 'NVIDIA', 'onsemi', 'Astera Labs', 'Etched')
  GROUP BY pc.user_id
  HAVING COUNT(*) >= 5
),
new_companies AS (
  SELECT id FROM companies
  WHERE display_name IN ('Marvell', 'Cadence', 'Ciena', 'Analog Devices', 'Microchip', 'General Motors',
                         'BlackBerry', 'Tenstorrent', 'Lightmatter', 'Cerebras', 'Kepler Communications')
),
tracked AS (
  INSERT INTO profile_companies (user_id, company_id)
  SELECT u.user_id, n.id FROM preset_users AS u CROSS JOIN new_companies AS n
  ON CONFLICT DO NOTHING
  RETURNING user_id
)
UPDATE profiles
SET evaluated_through = 0, filters_version = filters_version + 1
WHERE user_id IN (SELECT user_id FROM tracked);
