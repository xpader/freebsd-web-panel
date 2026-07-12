/**
 * Sleep for the given number of milliseconds.
 * @param {number} ms
 * @returns {Promise<void>}
 */
export function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

/**
 * Repeatedly call `checkFn` at `interval` until it returns true or `timeout`
 * elapses.  The first call happens after one `interval` delay (not immediately),
 * which gives the backend time to reflect the change.
 *
 * @param {() => Promise<boolean>} checkFn  async predicate — return true to stop
 * @param {{interval?: number, timeout?: number}} opts
 * @returns {Promise<boolean>} true if `checkFn` returned true, false on timeout
 */
export async function pollUntil(checkFn, { interval = 1500, timeout = 30000 } = {}) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    await sleep(interval);
    if (await checkFn()) return true;
  }
  return false;
}
