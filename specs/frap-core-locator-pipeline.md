# Plan: единый конвейер локаторов CLASSIFY→SCOPE→LOCATE→VERIFY→HEAL в frap-core

## Task Description

Финальная итерация качества PageObject в движке `frap-core` (Rust), поверх ветки
`feat/frap-core-semantic-locators` (две предыдущие доработки уже влиты: каскад + uniqueness confidence +
энтропия + семантический нейминг + DTO `value`; затем getBy* вывод + валидные идентификаторы + рефайн
кластеризации + escaping). Этот план **заменяет** черновик `specs/frap-core-semantic-salvage-locators.md`
и расширяет его до единой архитектуры выбора локатора.

Живой прогон 8 страниц Сбера выявил **две оставшиеся проблемы** (обе важны, обе решаются здесь):
- **P1 — дубль-текст/href-ссылки падают в позиционный CSS.** 87% остаточных позиционных — это `<a>` с
  текстом, чьи (role,name)/href НЕ уникальны глобально (одно меню, отрендеренное в header+footer+скрытом
  мобильном). Они получают `page.locator("ul > li:nth-of-type(6) > …")` вместо `getByText`.
- **P2 — безымянные повторяющиеся контролы** (карусель/иконки, ~12%) — нет текста/имени, позиционный
  неизбежен поэлементно, но они образуют повторяющийся компонент → им нужен relative-локатор контейнера.

Корневой механизм (подтверждён кодом): `finalize()` (element_map.rs ~258–288) выбирает **первый уникальный
кандидат в порядке каскада** (`candidates.find(|c| c.count <= 1)`), а НЕ кандидата с макс. base. Позиционный
fallback всегда `count: 1` (уникален по построению) и стоит последним → когда все именованные не-уникальны,
`find` пропускает их и садится на позиционный. Плюс уникальность считается **глобально** по странице, тогда
как «Купить» уникальна внутри своей карточки.

## Objective

Каждый интерактивный элемент получает **самый устойчивый из возможных** локатор через явный конвейер:
повторяющиеся компоненты — relative-локатор контейнера (`getByRole(container).filter(hasText(variable))`),
дубль-элементы — scoped/`.first()` семантика, уникальные — прямой `getByRole/getByText/getByLabel`,
позиционный CSS — только абсолютный last resort для безымянных. Измеримо на 8 снапшотах `.frap-analysis/`:
позиционных (`css`+`nth`) ≤10% live / ≤5% на фикстуре, conf≥0.8 ≥85% на фикстуре, List-кластеры эмитят
relative-локатор (без `.nth(index)` там, где есть variable-различитель), 0 имён с цифры, сгенерированный
PageObject компилируется.

## Problem Statement

Текущий движок выбирает стратегию по «первому уникальному в порядке каскада» с **глобальной** уникальностью,
из-за чего: (а) позиционный CSS (уникальный по построению) обгоняет любую не-уникальную семантику —
дубль-ссылки меню становятся хрупкими `nth-of-type`; (б) порядок каскада ≠ приоритет по стабильности
(латентная priority-инверсия: кнопка с текстом может дать `getByText` вместо более устойчивого
`getByRole`); (в) повторяющиеся компоненты (карточки/списки) выводятся как `page.locator(sample).nth(index)`
вместо устойчивого relative-локатора по variable-различителю; (г) кластеризация по `SemanticKey` и
структурная similarity в `signature`/healing работают раздельно, не подтверждая друг друга.

## Solution Approach

Свести выбор локатора в **единый конвейер из 5 стадий** (организующий принцип; HEAL уже существует):

1. **CLASSIFY** — какие элементы «одного вида», где повторяющиеся компоненты и их контейнеры. Совместить
   `SemanticKey`-кластеризацию со структурной similarity (`signature::calculate_confidence`): совпадение
   двух сигналов → твёрдый повторяющийся компонент; расхождение → одиночка. Для каждого кластера выделить
   **constant** атрибуты/сигналы (общие у всех членов → признак контейнера) и **variable** (различаются →
   различитель элемента); переиспользовать `href_path_template` (маскирование `*`) и энтропийный детектор.
   [combinations 5, 6]
2. **SCOPE** — для элемента определить ближайший **стабильный контейнер** (контейнер кластера-карточки /
   список / форма / landmark с `#id`/`aria-label`). Источник идентичности контейнера — новое поле
   `scope_hint` из коллектора.
3. **LOCATE** — каскад **внутри scope**: уникальность считать ОТНОСИТЕЛЬНО контейнера, не всей страницы
   [combination 1]. Порядок выбора — **по max base среди уникальных**, а не «первый по порядку» [combination
   3, чинит priority-инверсию]. Позиционный CSS — абсолютный пол (named > positional) [FIX 1]. `href="#"`/
   пустой → как нет-href.
4. **VERIFY** — движок подтверждает, что получившийся scoped/relative-локатор **однозначен глобально**
   (индексами, без браузера): scope+inner матчит ровно 1 (или намеренные N для list). Если нет — деградация
   ниже по каскаду. Не эмитить локатор, не прошедший VERIFY.
5. **HEAL** — signature-similarity как runtime safety net (уже есть, не трогаем).

**Реальное a11y-дерево в коллекторе** [combination 4]: `snapshot.js` (page-context, живой DOM) вычисляет
computed `role`, accessible `name` (с `aria-label`/`aria-labelledby`/`label[for]`/`alt`/`title`/текст),
`visible`, `scope_hint`. Один источник истины и для каскада (точный role+name), и для CLASSIFY (однородность
по роли). `DOMElementInfo` получает эти поля `#[serde(default)]` (старые снапшоты без них — деградация).

**Генерация** [combinations 2, FIX 2/3]: List-кластер с variable-различителем →
`page.getByRole(<containerRole>).filter(new Locator.FilterOptions().setHasText(<variable>)).getByRole(<childRole>)`;
scoped → `page.locator(<scope>).getByText/getByRole(<name>)`; не-уникальный → `.getByX(...).first()` +
комментарий `/** matches N */`. Валидные идентификаторы и экранирование сохранить.

**DTO (обратносовместимо):** `LocatorRecommendation` (Rust+Java) получает опциональные
`scope: Option<String>`, `filter_text: Option<String>` (variable-различитель для relative-локатора),
`match_count: Option<u32>`. `value` уже есть. Порядок/имена старых полей не трогаем.

## Relevant Files

Use these files to complete the task:

- `sdk/java/frap-mcp/frap-mcp-tools/src/main/resources/frap/snapshot.js` — collector: computed `role`,
  accessible `name`, `visible`, `scope_hint` (page-context, без сети).
- `crates/healing/src/lib.rs` — `DOMElementInfo` + `#[serde(default)]` поля `visible`, `scope_hint`,
  `computed_role`, `accessible_name`.
- `crates/signature/src/lib.rs` — переиспользовать `calculate_confidence`/структурную similarity в CLASSIFY;
  энтропийный детектор для variable/constant; возможно helper.
- `crates/core/src/element_map.rs` — CLASSIFY (SemanticKey↔similarity, variable/constant), SCOPE, LOCATE
  (max-base-among-unique, scoped uniqueness, named>positional), VERIFY; `LocatorRecommendation` поля
  `scope`/`filter_text`/`match_count`; `Cluster` обогащение (containerRole/childRole/variable).
- `crates/core/src/page_object.rs` — relative-локатор для List, scoped getBy*, `.first()`, валидные имена.
- `sdk/java/frap-core-java/src/main/java/io/github/kotlerdev/frap/core/dto/LocatorRecommendation.java` —
  поля `scope`, `filter_text`, `match_count`.
- `crates/core/tests/contract_locator_quality.rs` — расширить контракт.
- `crates/core/tests/fixtures/locator-quality/snap-person.json`, `.../expected.json` — фикстура/пороги
  (a11y/visible/scope_hint поля, дубль- и list-сценарии).
- `examples/java/playwright/src/test/java/io/github/kotlerdev/frap/demo/locator/GeneratedLocatorResolutionE2eTest.java`
  — e2e: relative/scoped/visible резолв + компиляция.
- **MUST-UPDATE — позиционные конструкторы `new LocatorRecommendation(...)`** при добавлении полей:
  `sdk/java/frap-mcp/frap-mcp-tools/src/test/java/io/github/kotlerdev/frap/mcp/tools/io/ArtifactStoreTest.java`,
  `sdk/java/frap-mcp/frap-mcp-tools/src/test/java/io/github/kotlerdev/frap/mcp/tools/FrapFileToolsTest.java`.

### New Files

- (нет обязательных новых файлов; всё расширяет существующие.)

## Implementation Phases

### Phase 1: Foundation (CLASSIFY + данные коллектора)
Коллектор отдаёт computed role / accessible name / visible / scope_hint (combination 4). CLASSIFY:
SemanticKey↔similarity-сверка + variable/constant-разметка кластера (combinations 5, 6), обогащение
`Cluster` (containerRole, childRole, variable-источник).

### Phase 2: Core Implementation (SCOPE + LOCATE + VERIFY)
Переписать `finalize` на «max base среди уникальных» (combination 3), уникальность относительно scope
(combination 1), named>positional (FIX 1), uniqueness среди видимых (FIX 4), VERIFY-гейт глобальной
однозначности. Заполнять `scope`/`filter_text`/`match_count`. DTO-зеркало (Java).

### Phase 3: Integration & Polish (генерация + тесты)
Relative-локатор для List (combination 2), scoped getBy*, `.first()`. Unit/contract/e2e; финальная
валидация + driver-гейт на 8 снапшотах.

## Team Orchestration

- You operate as the team lead and orchestrate the team to execute the plan.
- You're responsible for deploying the right team members with the right context to execute the plan.
- IMPORTANT: You NEVER operate directly on the codebase. You use `Task` and `Task*` tools to deploy team members.

### Team Members

- Builder
  - Name: builder-collector
  - Role: `snapshot.js` a11y/visible/scope_hint + `DOMElementInfo` поля
  - Agent Type: builder
  - Resume: false
- Builder
  - Name: builder-classify
  - Role: CLASSIFY в `element_map.rs` (SemanticKey↔similarity, variable/constant, обогащение Cluster)
  - Agent Type: builder
  - Resume: false
- Builder
  - Name: builder-locate
  - Role: SCOPE+LOCATE+VERIFY в `element_map.rs` (max-base-among-unique, scoped uniqueness, named>positional, поля DTO)
  - Agent Type: builder
  - Resume: false
- Builder
  - Name: builder-generator
  - Role: `page_object.rs` relative/scoped/.first()
  - Agent Type: builder
  - Resume: false
- Builder
  - Name: builder-dto
  - Role: Java `LocatorRecommendation` + call-sites
  - Agent Type: builder
  - Resume: false
- Builder
  - Name: builder-tests
  - Role: тесты по слоям (Rust unit, Rust integration, Java Playwright e2e)
  - Agent Type: builder
  - Resume: false
- Validator
  - Name: validator-final
  - Role: финальная валидация всех слоёв, driver-гейт, критерии приёмки
  - Agent Type: validator
  - Resume: false

## Testing Strategy

Test pyramid ratio: **80% unit / 15% integration-API / 5% UI e2e**

### Unit Tests (80%)
- `element_map.rs`: max-base-among-unique (priority-инверсия устранена), role-first для кнопки с текстом,
  named>positional, `href="#"`→no-href, scoped uniqueness внутри контейнера, VERIFY отбрасывает
  глобально-неоднозначный scoped, variable/constant-разметка кластера, CLASSIFY-сверка
  SemanticKey↔similarity, uniqueness среди видимых.
- `page_object.rs`: relative `filter(hasText).getByRole` для List, scoped getBy*, `.first()` для match_count>1.
- `healing`: a11y/visible/scope_hint десериализуются (есть/нет — обратная совместимость).

### Integration / API Tests (15%)
- `contract_locator_quality.rs` (расширение) над фикстурой с a11y/visible/scope_hint и дубль/list-сценариями:
  доля позиционных ≤ цель; List-кластеры → relative (нет `.nth(index)` где есть variable); дубль-текст →
  scoped/`.first()` (не позиционный); conf≥0.8 ≥ цель; все имена методов валидны. Существующие `frap-mcp` IT
  зелёные.

### UI E2E Tests (5%)
- `GeneratedLocatorResolutionE2eTest` (расширение): HTML с повторяющимся списком (карточки с разным текстом),
  дублированным меню (видимый + `display:none`, header vs footer). relative-локатор резолвит каждый item по
  имени; scoped/visible-уникальный getByText резолвится в 1 узел; PageObject компилируется (нет имён с цифры).

## Test Infrastructure (User-Declared)

### Unit Layer (Rust)
- **Files glob:** `crates/core/src/element_map.rs`, `crates/core/src/page_object.rs`, `crates/healing/src/lib.rs`
- **Infra signature:** `#\[test\]`
- **Happy-path scenarios:**
  - `finalize_prefers_max_base_among_unique`
  - `role_first_button_with_text_uses_role`
  - `named_candidate_outranks_positional_css`
  - `hash_href_treated_as_no_href`
  - `scoped_uniqueness_resolves_within_container`
  - `verify_rejects_globally_ambiguous_scoped_locator`
  - `cluster_exposes_variable_and_constant_split`
  - `classify_reconciles_semantickey_with_structural_similarity`
  - `uniqueness_computed_among_visible_only`
  - `generator_emits_relative_filter_locator_for_list`
  - `generator_emits_first_for_match_count_above_one`
  - `dom_element_info_a11y_fields_default_when_absent`
- **Runner command:** `cargo test -p frap-core -p healing -p signature`
- **Realism rationale:** чистые детерминированные функции движка/генератора на синтетических
  `DOMElementInfo`/`ElementMap` без I/O — точное покрытие конвейера CLASSIFY→SCOPE→LOCATE→VERIFY и вывода.

### Integration Layer (Rust)
- **Files glob:** `crates/core/tests/contract_locator_quality.rs`
- **Infra signature:** `generate_page_object\(`
- **Happy-path scenarios:**
  - `real_snapshot_positional_share_below_target`
  - `list_clusters_emit_relative_not_nth`
  - `dup_text_links_get_scoped_or_first_not_positional`
  - `named_elements_get_semantic_not_positional`
  - `real_snapshot_high_confidence_share_not_regressed`
- **Runner command:** `cargo test -p frap-core --test contract_locator_quality`
- **Realism rationale:** реальный движок + генератор над реальным (trimmed) снапшотом с a11y/visible/
  scope_hint — тот же путь, что `frap-mcp` в проде; метрики сверяются с порогами `expected.json`.

### E2E Layer (Java)
- **Status:** Enabled
- **Files glob:** `examples/java/playwright/src/test/java/io/github/kotlerdev/frap/demo/locator/GeneratedLocatorResolutionE2eTest.java`
- **Infra signature:** `Playwright\.create\(|getBy`
- **Happy-path scenarios:**
  - `relative_list_locator_resolves_each_item_by_name`
  - `scoped_and_visible_unique_locators_resolve_to_single_element`
  - `generated_pageobject_compiles_without_digit_leading_names`
- **Runner command:** `mvn -f examples/java/playwright/pom.xml test -Dtest=GeneratedLocatorResolutionE2eTest`
- **Realism rationale:** реальный HTML (повторяющийся список + `display:none` дубль) в настоящем headless
  Chromium доказывает, что relative/scoped/visible-локаторы реально резолвятся и код компилируется.

## Step by Step Tasks

### 1. Collector: a11y-дерево + visible + scope_hint

- **Task ID**: collector-a11y
- **Depends On**: none
- **Assigned To**: builder-collector
- **Agent Type**: builder
- **Stack**: Rust frap-core signature
- **Parallel**: false
- **Tests**: Unit (healing): `dom_element_info_a11y_fields_default_when_absent`.
- В `snapshot.js` (page-context, живой DOM, без сети) для каждого элемента добавить: `visible`
  (`getClientRects().length>0 || offsetParent!==null`); `computed_role` (явный `role` атрибут, иначе implicit
  из тега: a→link, button→button, input[type]→textbox/checkbox/radio, textarea→textbox, select→combobox,
  nav→navigation, и т.п.); `accessible_name` (по приоритету: `aria-label` → `aria-labelledby`-резолв (собрать
  текст ссылаемых id) → `label[for]`/обёртка-label → `alt` → `title` → видимый текст; нормализованный);
  `scope_hint` (ближайший предок-landmark `header/footer/nav/main/aside/form` как валидный CSS:
  `tag` + `#id` (CSS.escape) + `[aria-label="…"]` если есть, иначе `tag`; подъём по parentElement
  **до первого landmark-предка с ограничением ≤25 уровней** (избегать патологического обхода глубоких SPA;
  нет landmark в пределах лимита → `null`). Не ломать существующий формат/прочие поля.
- В `crates/healing/src/lib.rs` `DOMElementInfo` добавить `#[serde(default)]` поля `visible: Option<bool>`,
  `scope_hint: Option<String>`, `computed_role: Option<String>`, `accessible_name: Option<String>`
  (отсутствие = деградация, обратная совместимость со старыми снапшотами).

### 2. CLASSIFY: SemanticKey↔similarity + variable/constant

- **Task ID**: classify-clusters
- **Depends On**: collector-a11y
- **Assigned To**: builder-classify
- **Agent Type**: builder
- **Stack**: Rust frap-core element_map clustering
- **Parallel**: false
- **Tests**: Unit (element_map.rs): `cluster_exposes_variable_and_constant_split`, `classify_reconciles_semantickey_with_structural_similarity`.
- В `build_element_map` (CLASSIFY): кластеризацию `SemanticKey` подтверждать структурной similarity
  (`signature::calculate_confidence`/path-token-structural) — кластер считается твёрдым повторяющимся
  компонентом, когда оба сигнала согласны (порог); расхождение → `Single`. Использовать `computed_role` для
  однородности по роли (а не только по тегу).
- Для каждого List-кластера выделить **variable** (различается у членов: текст/`href_path_template` `*`-сегмент)
  и **constant** (общее: containerRole/childRole, общий путь). Энтропийный детектор (`signature`) отличает
  осмысленное от хеша. Обогатить Rust `Cluster` тремя полями с `#[serde(default)]` (фиксированный контракт
  для T5): `container_role: Option<String>`, `child_role: Option<String>`, `variable_kind: Option<String>`
  (`"text"|"href"`).
- **Java `Cluster` DTO зеркалить НЕ нужно:** `FrapRpcClient` (sdk/java/frap-core-java/.../client/) уже `.disable(FAIL_ON_UNKNOWN_PROPERTIES)` →
  новые Rust-поля молча игнорируются Java (генерация PageObject — в Rust, Java их не потребляет). 5 call-site'ов
  `new Cluster(...)` не трогать.
- Не ломать contract_element_map_list / contract_clustering_id_migration / build_element_map_groups_similar_paths.

### 3. SCOPE+LOCATE+VERIFY: каскад внутри контейнера

- **Task ID**: cascade-pipeline
- **Depends On**: classify-clusters
- **Assigned To**: builder-locate
- **Agent Type**: builder
- **Stack**: Rust frap-core element_map locator selector confidence
- **Parallel**: false
- **Tests**: Unit (element_map.rs): `finalize_prefers_max_base_among_unique`, `role_first_button_with_text_uses_role`, `named_candidate_outranks_positional_css`, `hash_href_treated_as_no_href`, `scoped_uniqueness_resolves_within_container`, `verify_rejects_globally_ambiguous_scoped_locator`, `uniqueness_computed_among_visible_only`.
- **LOCATE — порядок отбора (combination 3, чинит priority-инверсию):** переписать `finalize` (element_map.rs
  ~258–288): среди кандидатов с подтверждённой уникальностью выбирать **максимум по base** (а не «первый по
  порядку, `find(count<=1)`»). Позиционный fallback (`count:1`, ~444–450) ИСКЛЮЧИТЬ из прохода выбора
  уникальных — рассматривать только после исчерпания именованных.
  **Зафиксировать порядок base так, чтобы существующие тесты остались зелёными:**
  `testid 0.98 > href 0.92 > role 0.90 > placeholder 0.88 > text 0.80 > id 0.75 > css(позиционный) 0.40`.
  **Обновить именно КОНСТАНТЫ `BASE_*` (element_map.rs ~26–32): `BASE_HREF→0.92`, `BASE_ROLE→0.90`,
  `BASE_PLACEHOLDER→0.88`** (а не только переупорядочить логику) — `recommend_locator_single_element_has_degraded_confidence`
  ассертит против символа `BASE_HREF`, а `cascade_picks_href_when_unique` требует численно href > role.
  Это (а) чинит priority-инверсию `role > text` (кнопка с уникальным role+name → `getByRole`, не `getByText`),
  и (б) сохраняет `href > role`, чтобы уникальный href остался href.
  **Инвариант:** `уникальный именованный (max base) > не-уникальный именованный > позиционный`.
  **Держать зелёными / при необходимости синхронизировать ожидания:** `cascade_picks_href_when_unique`
  (уникальный href → href, т.к. href 0.92 > role 0.90), `recommend_locator_single_element_has_degraded_confidence`
  (одиночный API: уникальный href → href с base-confidence), `cascade_role_name_rescues_unique_text_without_attrs`
  (нет href, уникальный role+name → role), `confidence_drops_for_non_unique_candidate` (не-уникальный href
  обгоняет только позиционный, не уникальный role/text). Если поведение для какого-то теста меняется
  обоснованно — обновить ассерт и зафиксировать причину; по умолчанию порядок выше их сохраняет.
- **SCOPE-uniqueness (combination 1):** индекс `(scope_hint, role, name)` и `(cluster_container, role, name)`;
  уникальность считать ОТНОСИТЕЛЬНО scope/контейнера. «Купить» не уникальна на странице, но уникальна внутри
  карточки → scoped/relative-кандидат. Заполнять `scope` (для scoped) и `filter_text` (variable-различитель
  для relative-list) в `LocatorRecommendation`.
- **FIX 1 / FIX 4:** named>positional; `href="#"`/пустой → no-href; уникальность среди ВИДИМЫХ
  (`visible != Some(false)`), предпочитать видимый кандидат.
- **VERIFY:** перед фиксацией scoped/relative-кандидата подтвердить индексами глобальную однозначность
  (scope+inner матчит ровно 1; для list — filter_text различает членов). Если не однозначно — деградация ниже
  по каскаду; не эмитить непрошедший VERIFY. `match_count` (None/1 уникален, >1 matches N) проставлять.
- Использовать `accessible_name`/`computed_role` из `DOMElementInfo`, если присутствуют (точные role+name).
- Расширить `LocatorRecommendation`: `#[serde(default)] scope`, `filter_text`, `match_count`. Публичные
  сигнатуры `recommend_locator`/`build_element_map` сохранить.

### 4. DTO: scope/filter_text/match_count (Java)

- **Task ID**: dto-pipeline-fields
- **Depends On**: cascade-pipeline
- **Assigned To**: builder-dto
- **Agent Type**: builder
- **Stack**: Java jackson record dto
- **Parallel**: true
- **Tests**: Unit (Java): десериализация `scope`/`filter_text`/`match_count` (есть/нет — совместимость).
- Java `LocatorRecommendation` record: `@JsonProperty("scope") String scope`,
  `@JsonProperty("filter_text") String filterText`, `@JsonProperty("match_count") Integer matchCount`
  (nullable, последними; порядок старых полей не менять).
- Обновить позиционные `new LocatorRecommendation(...)` в `ArtifactStoreTest.java` (3) и
  `FrapFileToolsTest.java` (3) — добавить новые аргументы (`null`). Найти ВСЕ вызовы, иначе модуль не
  компилируется.

### 5. Генерация: relative для List + scoped + .first()

- **Task ID**: generator-relative
- **Depends On**: cascade-pipeline
- **Assigned To**: builder-generator
- **Agent Type**: builder
- **Stack**: Rust frap-core page_object
- **Parallel**: true
- **Tests**: Unit (page_object.rs): `generator_emits_relative_filter_locator_for_list`, `generator_emits_first_for_match_count_above_one`.
- **Relative-локатор для List (combination 2):** если `Cluster` несёт container_role/child_role и
  variable-различитель — эмитить метод с параметром:
  `public Locator <name>(String text) { return page.getByRole(AriaRole.<CONTAINER>).filter(new Locator.FilterOptions().setHasText(text)).getByRole(AriaRole.<CHILD>); }`
  вместо `page.locator(sample).nth(index)`. Если контейнер/роль не выводятся — оставить текущий `.nth(index)`.
- **scoped:** `scope` присутствует → `page.locator("<scope>").getByText/getByRole/getByLabel(<value>)`
  (scope экранировать ТОЛЬКО `escape_java_string` — whitespace значим, без `normalize_whitespace`).
- **.first():** `match_count > 1` → добавить `.first()` + комментарий `/** matches N — уточните */`.
- Импорт `com.microsoft.playwright.options.AriaRole` и `Locator.FilterOptions` — только если используются.
  Сохранить валидные идентификаторы (`^[A-Za-z_]`), маппинг strategy→getBy*, экранирование/нормализацию text.
- Обновить существующий unit-ассерт, если он завязан на старую форму вывода.

### 6. Unit-тесты конвейера

- **Task ID**: unit-tests
- **Depends On**: collector-a11y, classify-clusters, cascade-pipeline, generator-relative, dto-pipeline-fields
- **Assigned To**: builder-tests
- **Agent Type**: builder
- **Stack**: Rust frap-core test
- **Parallel**: false
- **Tests**: см. `### Unit Layer (Rust)` — все 12 сценариев; добрать недостающее.
- Свести/дополнить unit-покрытие до полноты конвейера. Имена тестов — ровно как в `### Unit Layer (Rust)`.

### 7. Integration-контракт (расширение)

- **Task ID**: integration-tests
- **Depends On**: collector-a11y, classify-clusters, cascade-pipeline, generator-relative, dto-pipeline-fields
- **Assigned To**: builder-tests
- **Agent Type**: builder
- **Stack**: Rust frap-core integration test fixtures contract test cargo test --test
- **Parallel**: false
- **Tests**: см. `### Integration Layer (Rust)` — все 5 сценариев.
- Дополнить `snap-person.json`: (а) повторяющийся список (≥3 карточки/listitem с разным текстом → relative);
  (б) дубль-меню (одинаковый текст+href, одна `visible:true`, одна `visible:false`; и пара под landmark
  `#id` → scoped); (в) элемент с текстом-цифрой (валидность имени); (г) безымянный контрол. Проставить
  `computed_role`/`accessible_name`/`scope_hint`/`visible`. Состав зафиксировать комментарием.
- Переписать `expected.json`: total меняется → ВСЕ абсолютные пороги пересчитать от нового размера (правило
  «только ужесточение» — про проценты). Цели: позиционных ≤ ceil(0.05·total) на фикстуре; conf≥0.8 ≥
  floor(0.85·total); счётчики relative/scoped/first.
- **Re-baseline распределения стратегий:** max-base finalize мигрирует часть элементов `text`→`role` (где есть
  и уникальный текст, и уникальный role+name). Пересчитать комментарий-разбивку стратегий в шапке
  `contract_locator_quality.rs` (load-bearing документация) и любые пороги по стратегиям под новую раскладку.
- Сценарии (точные имена для check_test_layers): `real_snapshot_positional_share_below_target`,
  `list_clusters_emit_relative_not_nth`, `dup_text_links_get_scoped_or_first_not_positional`,
  `named_elements_get_semantic_not_positional`, `real_snapshot_high_confidence_share_not_regressed`.

### 8. E2E: relative/scoped/visible резолв (расширение)

- **Task ID**: e2e-tests
- **Depends On**: integration-tests
- **Assigned To**: builder-tests
- **Agent Type**: builder
- **Stack**: Java Playwright e2e page object
- **Parallel**: false
- **Tests**: см. `### E2E Layer (Java)` — все 3 сценария.
- Пересобрать release `frap-core-rpc`, запускать с `FRAP_CORE_BIN` на свежий бинарь. Расширить
  `GeneratedLocatorResolutionE2eTest`: HTML с повторяющимся списком (карточки `role=listitem` с разным
  текстом) + дублированным меню (видимый + `display:none`; header vs footer).
  `relative_list_locator_resolves_each_item_by_name` — сгенерированный relative-метод
  `getByRole(...).filter(setHasText(name)).getByRole(...)` резолвит каждый item по имени;
  `scoped_and_visible_unique_locators_resolve_to_single_element` — scoped getByText и видимый-уникальный
  getByText в 1 узел; `generated_pageobject_compiles_without_digit_leading_names` — `assertCompiles`
  (компилятор + classpath playwright 1.44.0) + имена `^[A-Za-z_]`. Падение компиляции — сигнал вернуться к
  генератору, тест не удалять.

### 9. Финальная валидация

- **Task ID**: validate-all
- **Depends On**: collector-a11y, classify-clusters, cascade-pipeline, dto-pipeline-fields, generator-relative, unit-tests, integration-tests, e2e-tests
- **Assigned To**: validator-final
- **Agent Type**: validator
- **Stack**: Rust frap-core · Java maven
- **Parallel**: false
- Прогнать каждый `Runner command`, убедиться что тесты исполнились (N≥число сценариев слоя).
  `cargo test --workspace` зелёный; `cargo clippy --workspace -- -D warnings` чисто; `cargo fmt --check`.
- Пересобрать release `frap-core-rpc` (обе macOS-арки), обновить бандл в `frap-core-java` ресурсах; полный
  реактор: `mvn -f sdk/java/frap-core-java/pom.xml clean install` + `mvn -f sdk/java/frap-mcp/pom.xml clean
  verify` (с `FRAP_CORE_BIN`).
- **Driver-гейт по 8 живым снапшотам** `.frap-analysis/snap-*.json`: жёстко — доля позиционных (`css`+
  `nth-of-type`) **>10% → провал**, имя метода с цифры → провал. (Старые снапшоты без a11y/visible/scope_hint
  → combinations 1/2/4 частично неактивны, но max-base + named>positional + relative-по-SemanticKey уже дают
  ≤10%.) conf≥0.8 на живых — репорт; строгий ≥85% — на committed-фикстуре. Предварительно один раз замерить
  фактическую долю позиционных по страницам.
- Запустить `check_test_layers.py` и `check_diff_scope.py`. Проверить критерии приёмки.

## Acceptance Criteria

- Позиционный CSS никогда не выбирается для элемента с именем (text/aria-label/accessible_name) или href —
  такой элемент получает `getByRole`/`getByText`/`getByLabel`/relative/scoped/`.first()`.
- Доля позиционных (`nth-of-type`) ≤10% на 8 живых снапшотах (driver-гейт) и ≤5% на committed-фикстуре (было
  ~14%).
- List-кластеры с variable-различителем эмитят relative-локатор
  (`getByRole(container).filter(setHasText(...)).getByRole(child)`), а не `page.locator(sample).nth(index)`.
- Priority-инверсия устранена: среди уникальных кандидатов выбирается max-base (кнопка с role+name →
  `getByRole`, не `getByText`); `confidence_drops_for_non_unique_candidate` зелёный.
- VERIFY: ни один эмитируемый scoped/relative-локатор не является глобально неоднозначным (или помечен
  `.first()` осознанно).
- Дубль-текст-ссылки (P1) → scoped/`.first()`, не позиционный; безымянные повторы (P2) → relative-локатор
  контейнера, где выводится.
- conf≥0.8 ≥85% на committed-фикстуре; **0** имён методов с цифры; PageObject компилируется (`assertCompiles`).
- `cargo test --workspace` + `cargo clippy --workspace -- -D warnings` + `cargo fmt --check` зелёные;
  `frap-core-java` + полный `frap-mcp` реактор зелёные; существующие contract/DTO/MCP IT не сломаны.

## Validation Commands

Execute these commands to validate the task is complete:

- `cd crates && cargo test --workspace` — все Rust unit + contract зелёные.
- `cd crates && cargo clippy --workspace -- -D warnings` — без предупреждений.
- `cd crates && cargo fmt --check` — формат чист.
- `cd crates && cargo test -p frap-core --test contract_locator_quality` — контракт конвейера зелёный.
- `FRAP_CORE_BIN=$(pwd)/crates/target/release/frap-core-rpc mvn -f sdk/java/frap-core-java/pom.xml clean install` — core-java.
- `FRAP_CORE_BIN=$(pwd)/crates/target/release/frap-core-rpc mvn -f sdk/java/frap-mcp/pom.xml clean verify` — MCP реактор.
- `mvn -f examples/java/playwright/pom.xml test -Dtest=GeneratedLocatorResolutionE2eTest` — e2e: relative/scoped/visible резолв + компиляция.
- `uv run --script .claude/hooks/validators/check_test_layers.py --plan specs/frap-core-locator-pipeline.md` — слои реалистичны (PASS).

## Notes

- Заменяет черновик `specs/frap-core-semantic-salvage-locators.md` (его FIX 1/2/3/4 вошли как стадии
  LOCATE/SCOPE и combinations 1/3/4). Объём: движок `frap-core` (Rust) + `snapshot.js` (collector) + слой
  генерации Java PageObject. БЕЗ Selenium/Cypress; DTO расширяется только опциональными полями.
- ROI-порядок реализации внутри Phase 2: combination (3) max-base → (1) scoped-uniqueness → (2) relative →
  (4) a11y → (5)/(6) classify-сверка+variable/constant. Combinations 5/6 ограничены: подтверждение кластера
  структурной similarity + variable/constant-разметка для relative-локатора (не открытый «merge двух
  систем»).
- `Locator.getByRole/getByText/getByLabel/filter/.first()` — валидный Playwright Java API (Locator несёт то
  же семейство getBy*, что и Page). Компиляция (`assertCompiles`) — реальный гейт корректности.
- На старых снапшотах `.frap-analysis/` поля a11y/visible/scope_hint отсутствуют → combinations 1/2/4
  частично неактивны; долю позиционных live-гейта вытягивают combination 3 (max-base) + FIX 1
  (named>positional) + relative-по-SemanticKey. Полную выгоду проверяют committed-фикстура (поля заданы) и
  e2e (живой Chromium).
- HEAL (signature-similarity) не трогаем — остаётся runtime safety net.
- **Follow-up (вне scope этого плана):** `Frap.discover` использует ТРЕТИЙ коллектор —
  `adapters/playwright-java/.../SnapshotBuilder.java` (свой инлайн-JS), который НЕ эмитит a11y/scope_hint,
  поэтому relative-локатор для List через `Frap.discover` пока деградирует в `.nth(index)`. Production-путь
  (frap-mcp file mode → frap-mcp-tools/snapshot.js) обогащён в этом плане. Корректность генерации relative
  доказана на integration-контракте (`list_clusters_emit_relative_not_nth`, продакшн-форма фикстуры) +
  прямом вызове движка + компиляции + браузерном резолве relative-формы (e2e). Перенос обогащения в
  SnapshotBuilder — отдельная задача.
- **P2 (relative-локатор для безымянных повторов) проверяется на committed-фикстуре + e2e**, НЕ на live
  driver-гейте (тот меряет только долю позиционных, не наличие relative-локаторов). На старых снапшотах без
  a11y/scope_hint relative-эмиссия частично деградирует в `.nth(index)` — это допустимая мягкая деградация,
  не провал.
- **Cluster round-trip:** новые Rust-поля `Cluster` безопасны для Java MCP — `FrapRpcClient` отключает
  `FAIL_ON_UNKNOWN_PROPERTIES`; полный `frap-mcp clean verify` в validate-all подтверждает, что MCP-слой
  десериализует обогащённый `ElementMap` без ошибок.
