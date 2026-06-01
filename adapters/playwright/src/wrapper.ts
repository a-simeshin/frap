import type { Locator, Page } from '@playwright/test';
import {
  HealingEngine,
  FrapConfig,
  createHealingEngine,
  DOMSnapshot,
  DOMElementInfo,
  HealResult,
  buildElementFoundDebugReport,
  writeDebugReport,
  type HealPolicy,
} from '@frap/sdk';
import type { WithFrapOptions } from './config';
import {
  buildSemantics,
  enrichDebugReport,
  getCurrentPlaywrightTestId,
  logHealSemantics,
  recordHealingEvent,
} from './healing-events';

interface FrapLocator extends Locator {
  __frap?: {
    originalLocator: Locator;
    healingEngine: HealingEngine;
    config: FrapConfig;
    lastHealResult?: HealResult;
  };
}

const recordedSignatures = new Map<string, any>();

export async function withFrap<T extends Locator>(
  locator: T,
  page: Page,
  config?: WithFrapOptions,
  testName?: string
): Promise<FrapLocator> {
  const { testInfo, ...frapConfig } = config ?? {};
  const fullConfig: FrapConfig = {
    minConfidence: 0.85,
    reportDir: './frap-reports',
    enableHealing: true,
    enableReporting: true,
    healPolicy: 'allow',
    ...frapConfig,
  };
  const healPolicy: HealPolicy = fullConfig.healPolicy ?? 'allow';
  const resolvedTestName = testName ?? getCurrentPlaywrightTestId(testInfo);

  const healingEngine = await createHealingEngine(fullConfig);

  // Pre-record signature if element exists (for future healing)
  const selectorStr = locator.toString();
  // Extract actual selector from "locator('...')" string
  const selectorMatch = selectorStr.match(/locator\(['"](.+)['"]\)/);
  const cleanSelector = selectorMatch ? selectorMatch[1] : selectorStr;
  
  if (!recordedSignatures.has(selectorStr)) {
    try {
      // Use page.evaluate for reliable check
      const exists = await page.evaluate((sel) => {
        return document.querySelector(sel) !== null;
      }, cleanSelector);
      
      if (exists) {
        const sig = await extractSignatureFromPage(page, cleanSelector);
        if (sig) {
          recordedSignatures.set(selectorStr, sig);
          console.log(`[frap] Pre-recorded signature for ${cleanSelector}`);
        }
      }
    } catch (e) {
      // Ignore errors during pre-recording
    }
  }

  const frapLocator: FrapLocator = new Proxy(locator, {
    get(target, prop, receiver) {
      const value = Reflect.get(target, prop, receiver);
      
      if (typeof value === 'function' && ['click', 'fill', 'check', 'uncheck', 'selectOption', 'press', 'type'].includes(prop as string)) {
        return async (...args: any[]) => {
          const selectorKey = locator.toString();
          // Extract actual selector from "locator('...')" string
          const selectorMatch = selectorKey.match(/locator\(['"](.+)['"]\)/);
          const cleanSelector = selectorMatch ? selectorMatch[1] : selectorKey;

          // Quick check: if element doesn't exist, skip Playwright's long timeout and go straight to healing
          const elementExists = await page.evaluate((sel) => {
            return document.querySelector(sel) !== null;
          }, cleanSelector);

          if (!elementExists) {
            console.log(`[frap] Element not found (quick check): ${cleanSelector}`);

            if (!fullConfig.enableHealing) {
              throw new Error(`Element not found: ${cleanSelector}`);
            }

            console.log('[frap] Attempting healing...');
            const snapshot = await buildSnapshotFromPage(page);
            console.log(`[frap] DOM snapshot built: ${snapshot.elements.length} elements`);

            let originalSig = recordedSignatures.get(selectorKey);
            if (!originalSig) {
              // No pre-recorded signature - construct one from the selector and snapshot for healing
              console.log('[frap] No pre-recorded signature, constructing from selector...');
              originalSig = constructSignatureFromSelector(cleanSelector, snapshot);
            }

            const healResult = healingEngine.heal(cleanSelector, originalSig, snapshot, resolvedTestName);
            const semantics = buildSemantics(
              healPolicy,
              healResult.healed,
              true,
              'selector_missing'
            );
            const enrichedResult: HealResult = { ...healResult, semantics };
            frapLocator.__frap!.lastHealResult = enrichedResult;

            recordHealingEvent(
              {
                healSessionName: resolvedTestName,
                originalSelector: cleanSelector,
                newSelector: healResult.healed ? healResult.selector : undefined,
                healed: healResult.healed,
                confidence: healResult.confidence,
                trigger: semantics.trigger,
                policy: semantics.policy,
                outcome: semantics.outcome,
              },
              fullConfig.reportDir,
              testInfo
            );
            logHealSemantics(semantics, cleanSelector, healResult.selector);
            if (fullConfig.debug) {
              enrichDebugReport(fullConfig.reportDir, semantics, resolvedTestName);
            }

            console.log(`[frap] Healing result: healed=${healResult.healed}, confidence=${healResult.confidence.toFixed(2)}, selector="${healResult.selector}"`);
            console.log(`[frap] Top candidates: ${healResult.top_candidates.length}`);
            healResult.top_candidates.forEach((c, i) => {
              console.log(`[frap]   Candidate ${i}: "${c.selector}" confidence=${c.confidence.toFixed(2)}`);
            });

            if (healResult.healed && healResult.selector) {
              const healedLocator = page.locator(healResult.selector);
              const healedMethod = Reflect.get(healedLocator, prop);
              if (typeof healedMethod === 'function') {
                return await healedMethod.apply(healedLocator, args);
              }
            }

            throw new Error(`Healing failed for ${cleanSelector}: confidence=${healResult.confidence.toFixed(2)}, threshold=${fullConfig.minConfidence}`);
          }

          // Element exists, proceed with normal action
          try {
            const result = await value.apply(target, args);
            if (fullConfig.debug) {
              const report = buildElementFoundDebugReport(resolvedTestName, cleanSelector);
              if (fullConfig.healPolicy === 'deny') {
                report.semantics = buildSemantics(healPolicy, false, false, 'selector_missing');
              }
              writeDebugReport(fullConfig.reportDir, report);
            }
            return result;
          } catch (error: any) {
            console.log(`[frap] Primary action failed: ${error.message}`);
            const semantics = buildSemantics(healPolicy, false, true, 'action_failed');
            recordHealingEvent(
              {
                healSessionName: resolvedTestName,
                originalSelector: cleanSelector,
                healed: false,
                confidence: 0,
                trigger: semantics.trigger,
                policy: semantics.policy,
                outcome: semantics.outcome,
              },
              fullConfig.reportDir,
              testInfo
            );
            logHealSemantics(semantics, cleanSelector);
            if (fullConfig.debug) {
              enrichDebugReport(fullConfig.reportDir, semantics, resolvedTestName);
            }
            throw error;
          }
        };
      }

      return value;
    },
  }) as FrapLocator;

  frapLocator.__frap = {
    originalLocator: locator,
    healingEngine,
    config: fullConfig,
  };

  return frapLocator;
}

async function buildSnapshotFromPage(page: Page): Promise<DOMSnapshot> {
  console.log('[frap] Building DOM snapshot (optimized)...');

  try {
    return await page.evaluate(() => {
      const elements: DOMElementInfo[] = [];
      // Only get interactive elements and elements with data-testid - much faster than querySelectorAll('*')
      const interactiveElements = document.querySelectorAll(
        'button, input, a, select, textarea, [data-testid], [data-id], li[id], [role="button"], [role="link"], [role="input"]'
      );

      interactiveElements.forEach((el) => {
        const attributes: Record<string, string> = {};
        const attrs = Array.from(el.attributes);
        for (const attr of attrs) {
          attributes[attr.name] = attr.value;
        }

        const path: string[] = [];
        let current: Element | null = el;
        while (current && current !== document.body) {
          const role = current.getAttribute('role');
          path.unshift(`${current.tagName.toLowerCase()}:${role || '-'}`);
          current = current.parentElement;
        }

        let selector: string;
        const testId = el.getAttribute('data-testid');
        const dataId = el.getAttribute('data-id');
        const tagName = el.tagName.toLowerCase();
        // Visible text only: innerText skips <style>/<script> and hidden nodes,
        // unlike textContent which leaks inline CSS/JS into the locator.
        const visibleText = (((el as HTMLElement).innerText ?? el.textContent) || '')
          .replace(/\s+/g, ' ')
          .trim();
        const ariaLabel = el.getAttribute('aria-label')?.trim();
        if (testId) {
          selector = `[data-testid="${testId}"]`;
        } else if (dataId) {
          selector = `${tagName}[data-id="${dataId}"]`;
        } else if (el.id) {
          selector = `${tagName}[id="${el.id}"]`;
        } else if (ariaLabel) {
          selector = `${tagName}[aria-label="${ariaLabel.replace(/"/g, '\\"')}"]`;
        } else if (visibleText) {
          // Valid Playwright text pseudo-class, NOT the jQuery `:contains`.
          const label = visibleText.substring(0, 40).replace(/"/g, '\\"');
          selector = `${tagName}:has-text("${label}")`;
        } else {
          selector = tagName;
        }

        let positionInParent: number | undefined;
        const parent = el.parentElement;
        if (parent) {
          const siblings = Array.from(parent.children).filter(
            (child) => child.tagName === el.tagName
          );
          const idx = siblings.indexOf(el);
          if (idx >= 0) positionInParent = idx;
        }

        elements.push({
          selector,
          tag: tagName,
          attributes,
          text_content: visibleText ? visibleText.substring(0, 100) : undefined,
          path,
          position_in_parent: positionInParent,
        });
      });

      return {
        html: document.documentElement?.outerHTML?.substring(0, 1000) || '',
        elements,
      };
    });
  } catch (e) {
    console.error('[frap] Failed to build DOM snapshot:', e);
    // Return empty snapshot as fallback
    return {
      html: '',
      elements: [],
    };
  }
}

async function extractSignatureFromPage(page: Page, selector: string): Promise<any> {
  try {
    return await page.evaluate((sel) => {
      const el = document.querySelector(sel);
      if (!el) return null;

      const attributes: Record<string, string> = {};
      const attrs = Array.from(el.attributes);
      for (const attr of attrs) {
        attributes[attr.name] = attr.value;
      }

      const path: string[] = [];
      let current: Element | null = el;
      while (current && current !== document.body) {
        const role = current.getAttribute('role');
        path.unshift(`${current.tagName.toLowerCase()}:${role || '-'}`);
        current = current.parentElement;
      }

      const stableKeys = ['role', 'type', 'placeholder', 'aria-label', 'name'];
      const stable_attrs: Record<string, string> = {};
      for (const key of stableKeys) {
        if (attributes[key]) {
          stable_attrs[key] = attributes[key];
        }
      }
      if (attributes['data-testid']) stable_attrs['data-testid'] = attributes['data-testid'];
      if (attributes['id']) stable_attrs['id'] = attributes['id'];
      if (attributes['data-id']) stable_attrs['data-id'] = attributes['data-id'];

      const tokens = path.map((pathToken, i) => {
        const parts = pathToken.split(':');
        return {
          tag: parts[0] || 'unknown',
          role: parts[1] && parts[1] !== '-' ? parts[1] : undefined,
          depth: i,
        };
      });

      return {
        path: tokens,
        prefix: tokens.slice(0, 5).map((t: any) => `${t.tag}:${t.role || '-'}`).join('>'),
        stable_attrs,
        // innerText skips <style>/<script>, unlike textContent.
        text_content: ((el as HTMLElement).innerText ?? el.textContent) || undefined,
        children_hash: 0,
        depth: tokens.length,
      };
    }, selector);
  } catch (e) {
    console.error('[frap] Failed to extract signature:', e);
    return null;
  }
}

/**
 * Construct a signature from a selector and DOM snapshot.
 * This allows healing to work even for never-before-seen selectors.
 * Uses snapshot to find similar elements by data-testid pattern.
 */
function constructSignatureFromSelector(selector: string, snapshot: DOMSnapshot): any {
  // Parse data-testid selector: [data-testid="value"]
  const testIdMatch = selector.match(/\[data-testid=["']([^"']+)["']\]/);
  if (testIdMatch) {
    const originalTestId = testIdMatch[1];

    // Try to find similar elements in snapshot by data-testid pattern matching
    // This handles cases like "pay-btn" -> "checkout-pay"
    const similarElements = snapshot.elements.filter(el => {
      const elTestId = el.attributes['data-testid'];
      if (!elTestId) return false;

      // Check for substring overlap or semantic similarity
      const originalParts = originalTestId.toLowerCase().split(/[-_]/);
      const elParts = elTestId.toLowerCase().split(/[-_]/);

      // Match if they share any significant word parts
      return originalParts.some(part =>
        part.length > 2 && elParts.some(elPart => elPart.includes(part) || part.includes(elPart))
      );
    });

    // If we found similar elements, use the first one to build signature
    if (similarElements.length > 0) {
      const candidate = similarElements[0];
      const pathTokens = candidate.path.map((p, i) => {
        const parts = p.split(':');
        return { tag: parts[0] || 'unknown', role: parts[1] !== '-' ? parts[1] : undefined, depth: i };
      });

      return {
        path: pathTokens,
        prefix: pathTokens.slice(0, 5).map((t: any) => `${t.tag}:${t.role || '-'}`).join('>'),
        stable_attrs: { 'data-testid': originalTestId },
        text_content: candidate.text_content,
        children_hash: 0,
        depth: pathTokens.length,
      };
    }

    // No similar elements found - use any interactive element from snapshot
    // This ensures we still return candidates even with low confidence
    const interactiveElements = snapshot.elements.filter(el =>
      ['button', 'input', 'a', 'select'].includes(el.tag)
    );

    if (interactiveElements.length > 0) {
      const candidate = interactiveElements[0];
      const pathTokens = candidate.path.map((p, i) => {
        const parts = p.split(':');
        return { tag: parts[0] || 'unknown', role: parts[1] !== '-' ? parts[1] : undefined, depth: i };
      });

      return {
        path: pathTokens,
        prefix: pathTokens.slice(0, 5).map((t: any) => `${t.tag}:${t.role || '-'}`).join('>'),
        stable_attrs: { 'data-testid': originalTestId },
        text_content: candidate.text_content,
        children_hash: 0,
        depth: pathTokens.length,
      };
    }

    // No interactive elements - return signature with original testid
    return {
      path: [],
      prefix: '',
      stable_attrs: { 'data-testid': originalTestId },
      text_content: undefined,
      children_hash: 0,
      depth: 0,
    };
  }

  // Parse attribute id selector: li[id="2"] or #id
  const attrIdMatch = selector.match(/\[id=["']([^"']+)["']\]/);
  if (attrIdMatch) {
    const idValue = attrIdMatch[1];
    const liCandidate = snapshot.elements.find(
      (el) => el.attributes['id'] === idValue || el.attributes['data-id'] === idValue
    );
    if (liCandidate) {
      const pathTokens = liCandidate.path.map((p, i) => {
        const parts = p.split(':');
        return { tag: parts[0] || 'unknown', role: parts[1] !== '-' ? parts[1] : undefined, depth: i };
      });
      return {
        path: pathTokens,
        prefix: pathTokens.slice(0, 5).map((t: any) => `${t.tag}:${t.role || '-'}`).join('>'),
        stable_attrs: { id: idValue },
        text_content: liCandidate.text_content,
        position_in_parent: liCandidate.position_in_parent,
        children_hash: 0,
        depth: pathTokens.length,
      };
    }
    return {
      path: [{ tag: 'li', depth: 0 }],
      prefix: 'li:-',
      stable_attrs: { id: idValue },
      text_content: undefined,
      position_in_parent: undefined,
      children_hash: 0,
      depth: 1,
    };
  }

  const idMatch = selector.match(/^#([a-zA-Z0-9_-]+)$/);
  if (idMatch) {
    return {
      path: [],
      prefix: '',
      stable_attrs: { id: idMatch[1] },
      text_content: undefined,
      children_hash: 0,
      depth: 0,
    };
  }

  // Parse class selector: .class
  const classMatch = selector.match(/^\.([a-zA-Z0-9_-]+)$/);
  if (classMatch) {
    return {
      path: [],
      prefix: '',
      stable_attrs: { 'class': classMatch[1] },
      text_content: undefined,
      children_hash: 0,
      depth: 0,
    };
  }

  // Default: return minimal signature with the selector as-is
  return {
    path: [],
    prefix: '',
    stable_attrs: {},
    text_content: undefined,
    children_hash: 0,
    depth: 0,
  };
}

export function getLastHealResult(locator: FrapLocator): HealResult | undefined {
  return locator.__frap?.lastHealResult;
}
