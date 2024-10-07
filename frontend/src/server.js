import express from 'express';
import bodyParser from 'body-parser';

import cors from 'cors';
import fs from 'fs';

import path from 'path';

const app = express();
const port = 3001;

app.use(bodyParser.json());
app.use(cors());

app.post('/locales/add/:lng/:ns', (req, res) => {
  console.log(`Missing translations for ${req.params.lng}/${req.params.ns}:`, req.body);

  console.log(`inserting translations for ${req.params.lng}/${req.params.ns}:`, req.body);

  const { lng, ns } = req.params;
  const localesDir = path.join(process.cwd(), 'public', 'locales', lng);
  const filePath = path.join(localesDir, `${ns}`);

  // Ensure the directory exists
  if (!fs.existsSync(localesDir)) {
    fs.mkdirSync(localesDir, { recursive: true });
  }

  // Read existing translations or create an empty object
  let translations = {};
  if (fs.existsSync(filePath)) {
    const fileContent = fs.readFileSync(filePath, 'utf8');
    translations = JSON.parse(fileContent);
  }
  Object.keys(req.body).forEach((key) => {
    const keys = key.split('.');
    let current = translations;
    for (let i = 0; i < keys.length - 1; i++) {
      if (!current[keys[i]]) {
        current[keys[i]] = {};
      }
      current = current[keys[i]];
    }
    current[keys[keys.length - 1]] = `TO_BE_TRANSLATED: ${key}`;
  });

  // Write updated translations back to file
  fs.writeFileSync(filePath, JSON.stringify(translations, null, 2), 'utf8');

  res.status(200).send('Translations saved');
});

app.listen(port, () => {
  console.log(`Mock server listening at http://localhost:${port}`);
});
