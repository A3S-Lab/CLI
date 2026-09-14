# Use OKF supply contract (USE-1)

Authority: [product maturity roadmap](../../../docs/a3s-code-product-maturity-optimization-roadmap.md) §P2 USE-1

## Policy

1. **Use owns supply.** OKF packages arrive only through signed Registry /
   Plugin Manager apply → capability generation N.
2. **Official feed may admit zero OKF packages.** That is a supply policy, not a
   host bug. Hosts must not invent ambient Knowledge to look “complete.”
3. **Hosts fail closed without a projected Knowledge surface.**
   `use_knowledge_search` is admitted only while Knowledge is projected for the
   Run. No package ⇒ no queryable OKF authority.
4. **`$okf` skill ≠ OKF package.** The builtin skill teaches compilation /
   workflow; it does not imply a Use Knowledge lease.
5. **Trust roots are host configuration** (`registries.acl` / TUF), never
   compiled into Code Core or chosen by package content.

## Proof (Effect / Contract)

| Case | Expected | Evidence |
| --- | --- | --- |
| No OKF package | Host OKF paths fail closed; no fake hits | Matrix: “Host Pass (fail-closed without package)” |
| Mock / local OKF installed | Query returns digest-bound hits under exact lease | Hermetic `use_registry::knowledge::*`, live UKS brew roots |
| Official registry empty of OKF | Product docs tell the truth; demos use mock/local supply | This contract + Use README preview language |
| Multi-surface reconcile | Healthy OKF stays queryable (USE-2) | Matrix UKS brew `UKS_COMPOSE=yes` |

## Refuse

| Claim | Why rejected |
| --- | --- |
| “OKF everywhere” while official feed has none | Supply fiction |
| Soft-skip live UKS as green | Not Effect |
| Core builtin that shells to `a3s-use` | Second mutation path |
| Treating `$okf` skill load as Knowledge lease | Different surfaces |

## Related

- [capability-expansion-implementation-path.md](../../../docs/capability-expansion-implementation-path.md)
- [a3s-use-component-platform.md](./a3s-use-component-platform.md)
- Matrix `$okf` / Use Knowledge must row
