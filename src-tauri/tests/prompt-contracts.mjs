import fs from 'node:fs';
import path from 'node:path';

export function readUtf8(filePath) {
  return fs.readFileSync(filePath, 'utf8');
}

export function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

export function assertFileExistsAndNotEmpty(filePath) {
  assert(fs.existsSync(filePath), `File does not exist: ${filePath}`);
  const content = readUtf8(filePath).trim();
  assert(content.length > 0, `File is empty: ${filePath}`);
  return content;
}

export function assertIncludesAll(content, expectedSnippets, label) {
  for (const snippet of expectedSnippets) {
    assert(content.includes(snippet), `${label} is missing required snippet: ${snippet}`);
  }
}

export function parseScenarioFile(filePath) {
  const raw = readUtf8(filePath);
  return JSON.parse(raw);
}

export function assertScenarioGroup(scenarios, groupName, minimumCount) {
  assert(Array.isArray(scenarios[groupName]), `Scenario group must be an array: ${groupName}`);
  assert(
    scenarios[groupName].length >= minimumCount,
    `Scenario group ${groupName} must contain at least ${minimumCount} items`
  );
}

export function renderQualityPrompt(template, input) {
  return template
    .replaceAll('{{text_raw}}', input.text_raw ?? '')
    .replaceAll('{{text_optimized}}', input.text_optimized ?? '')
    .replaceAll('{{text_english}}', input.text_english ?? '');
}

export function assertNoUnresolvedPlaceholders(content, label) {
  assert(!content.includes('{{'), `${label} still contains unresolved placeholders`);
  assert(!content.includes('}}'), `${label} still contains unresolved placeholders`);
}

export function assertQualityScenarioShape(scenario, label) {
  assert(typeof scenario.expectedDecision === 'string', `${label} must declare expectedDecision`);
  assert(
    scenario.expectedDecision === 'KEEP' || scenario.expectedDecision === 'DISCARD',
    `${label} expectedDecision must be KEEP or DISCARD`
  );
  assert(scenario.input && typeof scenario.input === 'object', `${label} input must be an object`);
  assert(typeof scenario.input.text_raw === 'string', `${label} input.text_raw must be a string`);
  assert(typeof scenario.input.text_optimized === 'string', `${label} input.text_optimized must be a string`);
  assert(typeof scenario.input.text_english === 'string', `${label} input.text_english must be a string`);
}

export function buildRepoPath(...segments) {
  return path.join('D:\\git\\streaming-speech', ...segments);
}

export function buildPromptTestsPath(...segments) {
  return path.join('D:\\git\\streaming-speech', 'src-tauri', 'tests', ...segments);
}
