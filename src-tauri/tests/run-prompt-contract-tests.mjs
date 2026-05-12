import {
  assertFileExistsAndNotEmpty,
  assertIncludesAll,
  assertNoUnresolvedPlaceholders,
  assertQualityScenarioShape,
  assertScenarioGroup,
  buildPromptTestsPath,
  buildRepoPath,
  parseScenarioFile,
  renderQualityPrompt,
} from './prompt-contracts.mjs';

const promptDir = buildRepoPath('docs', '2026-05-13-prompt-system-tests');
const testsDir = buildPromptTestsPath();

const promptFiles = {
  optimize: buildRepoPath('docs', '2026-05-13-prompt-system-tests', 'program-expression-optimization.system.md'),
  translate: buildRepoPath('docs', '2026-05-13-prompt-system-tests', 'chinese-translation.system.md'),
  quality: buildRepoPath('docs', '2026-05-13-prompt-system-tests', 'quality-filter.system.md'),
  design: buildRepoPath('docs', '2026-05-13-prompt-system-tests', 'prompt-system-tests.md'),
};

const scenarioFile = buildPromptTestsPath('prompt-scenarios.json');

const results = [];

function pass(message) {
  results.push(`PASS ${message}`);
}

function run() {
  const optimizePrompt = assertFileExistsAndNotEmpty(promptFiles.optimize);
  pass('program-expression-optimization.system.md exists and is non-empty');
  assertIncludesAll(
    optimizePrompt,
    ['JSON', 'text_optimized', '不要返回 Markdown', '不扩写', '保留这些技术标识', '不要擅自改写', '保持原意'],
    'optimization prompt'
  );
  pass('optimization prompt declares text_optimized JSON contract');

  const translatePrompt = assertFileExistsAndNotEmpty(promptFiles.translate);
  pass('chinese-translation.system.md exists and is non-empty');
  assertIncludesAll(
    translatePrompt,
    ['JSON', 'text_english', '不要返回 Markdown', '保留'],
    'translation prompt'
  );
  pass('translation prompt declares text_english JSON contract');

  const qualityPrompt = assertFileExistsAndNotEmpty(promptFiles.quality);
  pass('quality-filter.system.md exists and is non-empty');
  assertIncludesAll(
    qualityPrompt,
    [
      'decision',
      'confidence',
      'reason',
      'KEEP',
      'DISCARD',
      '{{text_raw}}',
      '{{text_optimized}}',
      '{{text_english}}',
      '不确定时优先 KEEP',
      '简短中文说明',
      '不要返回 Markdown'
    ],
    'quality prompt'
  );
  pass('quality filter prompt declares decision/confidence/reason contract');

  const designDoc = assertFileExistsAndNotEmpty(promptFiles.design);
  pass('prompt-system-tests.md exists and is non-empty');
  assertIncludesAll(
    designDoc,
    ['## 项目现状', '## 整体目标', '## 核心设计决策', '## 测试设计', '## 验证策略'],
    'design document'
  );
  pass('design document contains required sections');

  const scenarios = parseScenarioFile(scenarioFile);
  pass('prompt-scenarios.json is valid JSON');

  assertScenarioGroup(scenarios, 'optimize', 8);
  assertScenarioGroup(scenarios, 'translate', 5);
  assertScenarioGroup(scenarios, 'qualityFilter', 8);
  pass('scenario file contains required scenario groups and minimum counts');

  for (const [index, scenario] of scenarios.qualityFilter.entries()) {
    const label = `quality scenario ${index + 1} (${scenario.id})`;
    assertQualityScenarioShape(scenario, label);
    const rendered = renderQualityPrompt(qualityPrompt, scenario.input);
    assertNoUnresolvedPlaceholders(rendered, label);
  }
  pass('quality filter scenarios declare valid judgment contracts and render without unresolved placeholders');

  assertFileExistsAndNotEmpty(testsDir + '\\prompt-contracts.mjs');
  assertFileExistsAndNotEmpty(testsDir + '\\run-prompt-contract-tests.mjs');
  pass('test programs exist and are non-empty');
}

try {
  console.log('Prompt contract tests\n');
  run();
  for (const line of results) {
    console.log(line);
  }
  console.log('\nAll prompt contract tests passed.');
} catch (error) {
  console.error('FAIL', error instanceof Error ? error.message : String(error));
  process.exit(1);
}
