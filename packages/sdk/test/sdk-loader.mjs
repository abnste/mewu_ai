// SPDX-License-Identifier: MPL-2.0
// Resolve the real independently authored plugin imports to this SDK package;
// production QuickJS supplies the same module using its own module resolver.
export function resolve(specifier, context, nextResolve) {
  if (specifier === '@mewu/sdk') return { url: new URL('../index.mjs', import.meta.url).href, shortCircuit: true };
  return nextResolve(specifier, context);
}
