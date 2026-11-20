(() => {
  'use strict';

  const API_BASE = '/api/v1/housing';
  const STORAGE_KEY = 'foundation_housing_studio_prefs_v1';
  const MAX_LOG_ENTRIES = 300;
  const SSE_EVENT_KINDS = [
    'session_state',
    'session_online',
    'session_updated',
    'session_offline',
    'session_switched',
    'session_unlocked',
    'kit_granted',
    'estate_ensured',
    'estate_updated',
    'estate_reset',
    'estate_deleted',
    'preset_applied',
    'live_diff_applied',
    'snapshot_created',
    'snapshot_restored',
    'furniture_moved',
    'furniture_dyed',
    'furniture_added',
    'furniture_removed',
    'layout_changed',
  ];

  const DISTRICTS = {
    339: '안개빛 마을',
    340: '라벤더 안식처',
    341: '하늘잔 마루',
    641: '시로가네',
    979: '엠피레움',
  };

  const STYLE_LABELS = {
    'mist': '안개빛 마을',
    'lavender beds': '라벤더 안식처',
    'goblet': '하늘잔 마루',
    'shirogane': '시로가네',
    'empyreum': '엠피레움',
    'minimalist': '심플 (화이트)',
    'dark minimalist': '심플 (다크)',
  };

  const ZONE_LOCATIONS = {
    339: '안개빛 마을 거주구',
    340: '라벤더 안식처 거주구',
    341: '하늘잔 마루 거주구',
    641: '시로가네 거주구',
    979: '엠피레움 거주구',
    282: '안개빛 마을 실내 (소형)',
    283: '안개빛 마을 실내 (중형)',
    284: '안개빛 마을 실내 (대형)',
    342: '라벤더 안식처 실내 (소형)',
    343: '라벤더 안식처 실내 (중형)',
    344: '라벤더 안식처 실내 (대형)',
    345: '하늘잔 마루 실내 (소형)',
    346: '하늘잔 마루 실내 (중형)',
    347: '하늘잔 마루 실내 (대형)',
    649: '시로가네 실내 (소형)',
    650: '시로가네 실내 (중형)',
    651: '시로가네 실내 (대형)',
    980: '엠피레움 실내 (소형)',
    981: '엠피레움 실내 (중형)',
    982: '엠피레움 실내 (대형)',
    1249: '심플 실내 (소형)',
    1250: '심플 실내 (중형)',
    1251: '심플 실내 (대형)',
    1374: '다크 심플 실내 (소형)',
    1375: '다크 심플 실내 (중형)',
    1376: '다크 심플 실내 (대형)',
    384: '안개빛 마을 개인실',
    385: '라벤더 안식처 개인실',
    386: '하늘잔 마루 개인실',
    652: '시로가네 개인실',
    983: '엠피레움 개인실',
    573: '톱마스트 아파트 로비',
    574: '릴리 힐즈 아파트 로비',
    575: '술타나즈 브레스 아파트 로비',
    654: '코바이 고텐 아파트 로비',
    985: '잉글사이드 아파트 로비',
    608: '톱마스트 아파트 실내',
    609: '릴리 힐즈 아파트 실내',
    610: '술타나즈 브레스 아파트 실내',
    655: '코바이 고텐 아파트 실내',
    999: '잉글사이드 아파트 실내',
    128: '림사 로민사 상층 갑판',
    129: '림사 로민사 하층 갑판',
    130: '울다하 날 회랑',
    131: '울다하 달 회랑',
    132: '그리다니아 신시가지',
    133: '그리다니아 구시가지',
    418: '이슈가르드 하층',
    419: '이슈가르드 상층',
    628: '쿠가네',
    819: '크리스타리움',
    962: '올드 샬레이안',
    1185: '툴라이욜라',
  };

  const FIXTURE_LABELS = {
    wall: '벽지',
    flooring: '바닥재',
    ceiling: '천장 조명',
    light: '천장 조명',
    'upper wall': '2층 벽지',
    'upper flooring': '2층 바닥',
    'basement wall': '지하 벽지',
    'basement flooring': '지하 바닥',
    roof: '지붕',
    walls: '외벽',
    window: '창문',
    door: '출입문',
    fence: '울타리',
    awning: '차양',
    sign: '간판',
    'roof decoration': '지붕 장식',
    'wall decoration': '외벽 장식',
  };

  const state = {
    activeTab: 'estates',
    viewMode: 'cards',
    favoritesOnly: false,
    favorites: {},
    memos: {},
    primarySession: null,
    estates: [],
    selectedLandIdent: null,
    selectedEstateDetail: null,
    serverPresets: [],
    localPresets: [],
    selectedPresetId: null,
    bridgeSnapshot: null,
    furnScope: 'indoor',
    selectedFurnSlot: null,
    transformDraft: null,
    transformOriginal: null,
    liveMoveTimer: null,
    sseRefreshTimer: null,
    logs: [],
    logCategory: 'all',
    sseConnected: false,
  };

  const $ = (id) => document.getElementById(id);

  function loadPrefs() {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (!raw) return;
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed === 'object') {
        if (parsed.activeTab) state.activeTab = parsed.activeTab;
        if (parsed.viewMode === 'table' || parsed.viewMode === 'cards') state.viewMode = parsed.viewMode;
        if (parsed.favorites && typeof parsed.favorites === 'object') state.favorites = parsed.favorites;
        if (parsed.memos && typeof parsed.memos === 'object') state.memos = parsed.memos;
      }
    } catch {
      // Ignore corrupt localStorage entries.
    }
  }

  function savePrefs() {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({
        activeTab: state.activeTab,
        viewMode: state.viewMode,
        favorites: state.favorites,
        memos: state.memos,
      }));
    } catch {
      // Ignore storage quota issues.
    }
  }

  function esc(value) {
    return String(value ?? '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function utf8ByteLength(str) {
    return new TextEncoder().encode(str || '').length;
  }

  function toast(message, isError = false) {
    const stack = $('toastStack');
    const el = document.createElement('div');
    el.className = `toast ${isError ? 'err' : 'ok'}`;
    el.textContent = message;
    stack.appendChild(el);
    setTimeout(() => {
      el.remove();
    }, 3600);
  }

  function askConfirm(title, message) {
    return new Promise((resolve) => {
      const modal = $('confirmModal');
      $('confirmModalTitle').textContent = title;
      $('confirmModalMessage').textContent = message;
      modal.classList.remove('hidden');

      const cleanup = (answer) => {
        modal.classList.add('hidden');
        $('confirmModalOk').removeEventListener('click', onOk);
        $('confirmModalCancel').removeEventListener('click', onCancel);
        modal.removeEventListener('click', onBackdrop);
        resolve(answer);
      };
      const onOk = () => cleanup(true);
      const onCancel = () => cleanup(false);
      const onBackdrop = (ev) => {
        if (ev.target === modal) cleanup(false);
      };

      $('confirmModalOk').addEventListener('click', onOk);
      $('confirmModalCancel').addEventListener('click', onCancel);
      modal.addEventListener('click', onBackdrop);
    });
  }

  async function apiJson(path, options = {}) {
    const headers = { ...(options.headers || {}) };
    if (options.body !== undefined && !headers['Content-Type']) {
      headers['Content-Type'] = 'application/json';
    }
    const resp = await fetch(`${API_BASE}${path}`, {
      ...options,
      headers,
    });
    const text = await resp.text();
    let data = null;
    if (text) {
      try {
        data = JSON.parse(text);
      } catch {
        data = text;
      }
    }
    if (!resp.ok) {
      const errMsg = (data && typeof data === 'object' && data.error) ? data.error : (text || `HTTP ${resp.status}`);
      throw new Error(errMsg);
    }
    return data;
  }

  function formatDistrictName(tid) {
    return DISTRICTS[tid] || `구역 ${tid}`;
  }

  function formatSizeLabel(plotSize, isApartment) {
    if (isApartment) return '아파트';
    if (plotSize === 0) return '소형';
    if (plotSize === 1) return '중형';
    if (plotSize === 2) return '대형';
    return '기타';
  }

  function formatSizeCapacity(plotSize, isApartment) {
    if (isApartment) return { indoor: 100, outdoor: 0 };
    if (plotSize === 0) return { indoor: 300, outdoor: 40 };
    if (plotSize === 1) return { indoor: 450, outdoor: 60 };
    if (plotSize === 2) return { indoor: 600, outdoor: 80 };
    return { indoor: null, outdoor: null };
  }

  function formatStyleName(raw, fallback = '기본 양식') {
    if (!raw || !String(raw).trim()) return fallback;
    const lower = String(raw).trim().toLowerCase();
    if (lower.includes('dark') && lower.includes('minimal')) return '심플 (다크)';
    if (lower.includes('minimal')) return '심플 (화이트)';
    if (lower.includes('mist') || lower.includes('안개')) return '안개빛 마을';
    if (lower.includes('lavender') || lower.includes('라벤더')) return '라벤더 안식처';
    if (lower.includes('goblet') || lower.includes('하늘잔')) return '하늘잔 마루';
    if (lower.includes('shirogane') || lower.includes('시로')) return '시로가네';
    if (lower.includes('empyreum') || lower.includes('ishgard') || lower.includes('엠피')) return '엠피레움';
    return String(raw).trim();
  }

  function normalizeStyleKey(raw) {
    if (!raw) return 'lavender beds';
    const lower = String(raw).trim().toLowerCase();
    if (lower.includes('dark') && lower.includes('minimal')) return 'dark minimalist';
    if (lower.includes('minimal')) return 'minimalist';
    if (lower.includes('mist') || lower.includes('안개')) return 'mist';
    if (lower.includes('lavender') || lower.includes('라벤더')) return 'lavender beds';
    if (lower.includes('goblet') || lower.includes('하늘잔')) return 'goblet';
    if (lower.includes('shirogane') || lower.includes('시로')) return 'shirogane';
    if (lower.includes('empyreum') || lower.includes('ishgard') || lower.includes('엠피')) return 'empyreum';
    return 'lavender beds';
  }

  function formatEstateLocation(row) {
    const district = formatDistrictName(row.territory_type_id);
    if (row.is_apartment) {
      const divName = row.division !== 0 ? '확장구' : '본구';
      return `${district} ${row.ward_index + 1}구 ${divName} #${row.room_number}호`;
    }
    const humanPlot = row.division * 30 + row.plot_index + 1;
    return `${district} ${row.ward_index + 1}구 ${humanPlot}번지`;
  }

  function formatTimestamp(sec) {
    if (!sec) return '-';
    try {
      return new Date(sec * 1000).toLocaleString('ko-KR');
    } catch {
      return String(sec);
    }
  }

  function getSelectedEstate() {
    return state.estates.find((e) => e.row.land_ident === state.selectedLandIdent) || null;
  }

  function getAllPresets() {
    return [...state.localPresets, ...state.serverPresets];
  }

  function getSelectedPreset() {
    const all = getAllPresets();
    return all.find((p) => p.id === state.selectedPresetId) || null;
  }

  function decodeBufferWithBom(buffer) {
    const bytes = new Uint8Array(buffer);
    if (bytes.length >= 2 && bytes[0] === 0xff && bytes[1] === 0xfe) {
      return new TextDecoder('utf-16le').decode(bytes.subarray(2));
    }
    if (bytes.length >= 2 && bytes[0] === 0xfe && bytes[1] === 0xff) {
      return new TextDecoder('utf-16be').decode(bytes.subarray(2));
    }
    if (bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) {
      return new TextDecoder('utf-8').decode(bytes.subarray(3));
    }
    return new TextDecoder('utf-8').decode(bytes);
  }

  function normalizePresetSize(rawSize) {
    const s = String(rawSize || '').trim().toLowerCase();
    if (!s) return { plotSize: null, isApartment: false, badge: '미상' };
    if (s.includes('apart') || s.includes('chamber') || s.includes('fc') || s.includes('room')) {
      return { plotSize: 0, isApartment: true, badge: '아파트' };
    }
    if (s.includes('small') || s.includes('cottage')) return { plotSize: 0, isApartment: false, badge: '소형' };
    if (s.includes('medium') || s.includes('house')) return { plotSize: 1, isApartment: false, badge: '중형' };
    if (s.includes('large') || s.includes('mansion')) return { plotSize: 2, isApartment: false, badge: '대형' };
    return { plotSize: null, isApartment: false, badge: rawSize };
  }

  function analyzeFurnitureArray(arr) {
    let total = 0;
    let dyed = 0;
    let material = 0;
    const visit = (item) => {
      if (!item || typeof item !== 'object') return;
      total += 1;
      const props = item.properties;
      if (props && typeof props === 'object') {
        if ((typeof props.color === 'string' && props.color.trim()) || (typeof props.stainId === 'number' && props.stainId > 0)) {
          dyed += 1;
        }
        if (props.material && typeof props.material === 'object' && props.material.itemId > 0) {
          material += 1;
        }
      }
      if (Array.isArray(item.attachments)) {
        item.attachments.forEach(visit);
      }
    };
    if (Array.isArray(arr)) arr.forEach(visit);
    return { total, dyed, material };
  }

  function extractFixtureNames(arr, includeDistrict) {
    if (!Array.isArray(arr)) return [];
    const lines = [];
    for (const f of arr) {
      if (!f || typeof f !== 'object') continue;
      const kind = String(f.type || '').trim();
      const name = String(f.name || '').trim();
      if (!kind || !name) continue;
      const lower = kind.toLowerCase();
      if (lower === 'side door' || lower.includes('hidden')) continue;
      if (lower === 'district') {
        if (includeDistrict) lines.push(`구역: ${name}`);
        continue;
      }
      const label = FIXTURE_LABELS[lower] || kind;
      const color = f.color ? ` (${f.color})` : '';
      lines.push(`${label}: ${name}${color}`);
    }
    return lines;
  }

  function parseLocalPresetFile(fileName, rawJson, lastModifiedMs) {
    const id = `local:${fileName}`;
    try {
      const parsed = JSON.parse(rawJson);
      const sizeInfo = normalizePresetSize(parsed.houseSize);
      const intFixtures = Array.isArray(parsed.interiorFixture) ? parsed.interiorFixture : [];
      const extFixtures = Array.isArray(parsed.exteriorFixture) ? parsed.exteriorFixture : [];
      const districtFixture = intFixtures.find((f) => String(f?.type || '').toLowerCase() === 'district');
      const indoorStats = analyzeFurnitureArray(parsed.interiorFurniture);
      const outdoorStats = analyzeFurnitureArray(parsed.exteriorFurniture);

      return {
        id,
        sourceKind: '로컬',
        name: fileName,
        title: fileName.replace(/\.json$/i, ''),
        rawJson,
        serverPath: null,
        houseSizeRaw: parsed.houseSize || '',
        normalizedPlotSize: sizeInfo.plotSize,
        isApartment: sizeInfo.isApartment,
        sizeBadge: sizeInfo.badge,
        districtRaw: districtFixture?.name || '',
        styleKo: formatStyleName(districtFixture?.name, '양식 미지정'),
        indoorCount: indoorStats.total,
        indoorDyed: indoorStats.dyed,
        indoorMaterial: indoorStats.material,
        outdoorCount: outdoorStats.total,
        outdoorDyed: outdoorStats.dyed,
        interiorFixtures: extractFixtureNames(intFixtures, false),
        exteriorFixtures: extractFixtureNames(extFixtures, false),
        interiorScale: Number(parsed.interiorScale ?? 1.0),
        exteriorScale: Number(parsed.exteriorScale ?? 1.0),
        modifiedUnixMs: lastModifiedMs || Date.now(),
        parseError: null,
      };
    } catch (err) {
      return {
        id,
        sourceKind: '로컬',
        name: fileName,
        title: fileName.replace(/\.json$/i, ''),
        rawJson,
        serverPath: null,
        sizeBadge: '오류',
        normalizedPlotSize: null,
        isApartment: false,
        styleKo: '파싱 실패',
        indoorCount: 0,
        indoorDyed: 0,
        indoorMaterial: 0,
        outdoorCount: 0,
        outdoorDyed: 0,
        interiorFixtures: [],
        exteriorFixtures: [],
        interiorScale: 1,
        exteriorScale: 1,
        modifiedUnixMs: lastModifiedMs || Date.now(),
        parseError: err.message,
      };
    }
  }

  function mapServerPreset(summary) {
    const sizeInfo = normalizePresetSize(summary.house_size);
    return {
      id: `server:${summary.path}`,
      sourceKind: '서버',
      name: summary.name,
      title: String(summary.name || '').replace(/\.json$/i, ''),
      rawJson: null,
      serverPath: summary.path,
      houseSizeRaw: summary.house_size || '',
      normalizedPlotSize: sizeInfo.plotSize,
      isApartment: sizeInfo.isApartment,
      sizeBadge: sizeInfo.badge,
      districtRaw: summary.district || '',
      styleKo: formatStyleName(summary.district, '양식 미지정'),
      indoorCount: summary.indoor_count || 0,
      indoorDyed: 0,
      indoorMaterial: 0,
      outdoorCount: summary.outdoor_count || 0,
      outdoorDyed: 0,
      interiorFixtures: summary.interior_fixtures || [],
      exteriorFixtures: summary.exterior_fixtures || [],
      interiorScale: 1,
      exteriorScale: 1,
      modifiedUnixMs: summary.modified_unix_ms || 0,
      parseError: null,
    };
  }

  function evaluateCompat(preset, estate) {
    if (!estate) {
      return { status: 'neutral', text: '적용 대상 하우스를 먼저 선택하세요.', mismatch: false };
    }
    if (!preset) {
      return { status: 'neutral', text: '프리셋을 선택하거나 JSON 파일을 드롭하면 크기 호환성을 검사합니다.', mismatch: false };
    }
    if (preset.parseError) {
      return { status: 'warn', text: `JSON 파싱 오류: ${preset.parseError}`, mismatch: false };
    }

    const row = estate.row;
    const targetBadge = formatSizeLabel(row.plot_size, row.is_apartment);
    const cap = formatSizeCapacity(row.plot_size, row.is_apartment);

    if (preset.normalizedPlotSize === null) {
      return {
        status: 'neutral',
        text: `대상 [${targetBadge}] · 프리셋에 크기 정보 없음 (실내 ${preset.indoorCount} · 정원 ${preset.outdoorCount})`,
        mismatch: false,
      };
    }

    const sameType = preset.isApartment === row.is_apartment && (row.is_apartment || preset.normalizedPlotSize === row.plot_size);
    if (!sameType) {
      return {
        status: 'warn',
        mismatch: true,
        text: `크기 불일치: 프리셋은 [${preset.sizeBadge}] 전용이나 선택된 하우스는 [${targetBadge}]입니다. 맞는 크기 하우스를 선택하거나 [호환성 정밀 검증]을 확인하세요.`,
      };
    }

    if (cap.indoor !== null && preset.indoorCount > cap.indoor) {
      return {
        status: 'warn',
        mismatch: false,
        text: `크기 일치 [${targetBadge}], 단 실내 가구(${preset.indoorCount}개)가 상한(${cap.indoor}개)을 초과해 일부 잘릴 수 있습니다.`,
      };
    }

    return {
      status: 'match',
      mismatch: false,
      text: `크기 일치 [${targetBadge}] · 실내 ${preset.indoorCount}/${cap.indoor ?? '-'} · 정원 ${preset.outdoorCount}/${cap.outdoor ?? '-'} 적용 가능`,
    };
  }

  // ---- Navigation & Selection ----

  function switchTab(tabName) {
    state.activeTab = tabName;
    savePrefs();
    document.querySelectorAll('.nav-item').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.tab === tabName);
    });
    document.querySelectorAll('.tab-view').forEach((view) => {
      view.classList.toggle('active', view.id === `view-${tabName}`);
    });

    if (tabName === 'furniture' && state.selectedLandIdent) {
      refreshBridgeSnapshot();
    } else if (tabName === 'environment' && state.selectedLandIdent) {
      refreshEstateDetail();
    }
  }

  async function selectEstate(landIdent, loadSubViews = true) {
    state.selectedLandIdent = landIdent;
    renderAll();

    if (!landIdent || !loadSubViews) return;
    if (state.activeTab === 'furniture') {
      await refreshBridgeSnapshot();
    } else if (state.activeTab === 'environment') {
      await refreshEstateDetail();
    } else {
      // Load detail silently so snapshot count stays fresh in sidebar badge.
      await refreshEstateDetail({ silent: true });
    }
  }

  // ---- Data Fetching ----

  async function refreshAll(preferredLandIdent = null) {
    try {
      const [status, estates, presetsResp] = await Promise.all([
        apiJson('/status'),
        apiJson('/estates'),
        apiJson('/presets').catch(() => ({ files: [] })),
      ]);

      $('protocolPatchBadge').textContent = status.protocol_patch || '7.58';
      state.primarySession = Array.isArray(status.online_sessions) && status.online_sessions.length > 0
        ? status.online_sessions[0]
        : null;
      state.estates = Array.isArray(estates) ? estates : [];
      state.serverPresets = Array.isArray(presetsResp.files)
        ? presetsResp.files.map(mapServerPreset)
        : [];

      const desiredIdent = preferredLandIdent
        ?? state.selectedLandIdent
        ?? state.primarySession?.active_estate?.land_ident
        ?? state.estates[0]?.row.land_ident
        ?? null;

      if (desiredIdent !== null && state.estates.some((e) => e.row.land_ident === desiredIdent)) {
        state.selectedLandIdent = desiredIdent;
      } else {
        state.selectedLandIdent = state.estates[0]?.row.land_ident ?? null;
      }

      const allPresets = getAllPresets();
      if (!allPresets.some((p) => p.id === state.selectedPresetId)) {
        state.selectedPresetId = allPresets[0]?.id ?? null;
      }

      renderAll();

      if (state.selectedLandIdent) {
        if (state.activeTab === 'furniture') {
          await refreshBridgeSnapshot();
        } else {
          await refreshEstateDetail({ silent: true });
        }
      } else {
        state.selectedEstateDetail = null;
        state.bridgeSnapshot = null;
        renderAll();
      }
    } catch (err) {
      toast(`상태 불러오기 실패: ${err.message}`, true);
    }
  }

  async function refreshEstatesOnly() {
    try {
      const estates = await apiJson('/estates');
      state.estates = Array.isArray(estates) ? estates : [];
      if (!state.estates.some((e) => e.row.land_ident === state.selectedLandIdent)) {
        state.selectedLandIdent = state.primarySession?.active_estate?.land_ident
          ?? state.estates[0]?.row.land_ident
          ?? null;
      }
      renderAll();
      if (state.selectedLandIdent) {
        if (state.activeTab === 'furniture') await refreshBridgeSnapshot();
        if (state.activeTab === 'environment') await refreshEstateDetail({ silent: true });
      }
    } catch (err) {
      toast(`하우스 목록 갱신 실패: ${err.message}`, true);
    }
  }

  async function refreshEstateDetail({ silent = false } = {}) {
    if (!state.selectedLandIdent) {
      state.selectedEstateDetail = null;
      renderEnvironment();
      renderSidebarSelected();
      return;
    }
    try {
      const detail = await apiJson(`/estates/${state.selectedLandIdent}`);
      if (detail?.estate?.land_ident === state.selectedLandIdent) {
        state.selectedEstateDetail = detail;
        renderEnvironment();
        renderSidebarSelected();
      }
    } catch (err) {
      if (!silent) toast(`상세 조회 실패: ${err.message}`, true);
    }
  }

  async function refreshBridgeSnapshot() {
    if (!state.selectedLandIdent) {
      state.bridgeSnapshot = null;
      renderFurniture();
      return;
    }
    try {
      const snap = await apiJson(`/estates/${state.selectedLandIdent}/bridge/snapshot`);
      if (snap?.land_ident === state.selectedLandIdent) {
        state.bridgeSnapshot = snap;
        syncSelectedFurnitureIntoDraft();
        renderFurniture();
      }
    } catch (err) {
      toast(`가구 목록 조회 실패: ${err.message}`, true);
    }
  }

  // ---- SSE Connection ----

  function categorizeEventKind(kind) {
    if (!kind) return 'env';
    if (kind.startsWith('session_') || kind === 'kit_granted') return 'session';
    if (kind === 'preset_applied' || kind === 'live_diff_applied') return 'preset';
    if (kind.startsWith('furniture_') || kind === 'layout_changed') return 'furniture';
    return 'env';
  }

  function pushLog(evt) {
    state.logs.unshift({
      time: new Date().toLocaleTimeString('ko-KR', { hour12: false }),
      kind: evt.kind || 'info',
      category: categorizeEventKind(evt.kind),
      source: evt.source || '',
      revision: evt.revision ?? null,
      landIdent: evt.land_ident ?? null,
      message: evt.message || '',
    });
    if (state.logs.length > MAX_LOG_ENTRIES) {
      state.logs.length = MAX_LOG_ENTRIES;
    }
    renderLogs();
  }

  function handleSsePayload(rawJson) {
    let evt;
    try {
      evt = JSON.parse(rawJson);
    } catch {
      return;
    }
    setSseConnected(true);

    if (evt.kind === 'session_offline') {
      state.primarySession = null;
      renderSessionHeader();
      renderEstates();
      if (evt.message) pushLog(evt);
      return;
    }

    if (evt.session) {
      state.primarySession = evt.session;
      renderSessionHeader();
      renderEstates();
    }
    if (evt.message && (evt.kind !== 'furniture_moved' || evt.source)) {
      pushLog(evt);
    } else if (evt.kind === 'furniture_moved' && !evt.session) {
      pushLog(evt);
    }

    const shouldRefreshEstates = [
      'estate_ensured',
      'estate_updated',
      'estate_reset',
      'estate_deleted',
      'preset_applied',
      'live_diff_applied',
      'snapshot_created',
      'snapshot_restored',
      'session_switched',
      'furniture_added',
      'furniture_removed',
      'layout_changed',
    ].includes(evt.kind);

    if (shouldRefreshEstates || evt.kind === 'furniture_moved' || evt.kind === 'furniture_dyed') {
      clearTimeout(state.sseRefreshTimer);
      state.sseRefreshTimer = setTimeout(() => {
        refreshEstatesOnly();
      }, 220);
    }
  }

  function setSseConnected(connected) {
    state.sseConnected = connected;
    $('sseDot').className = `dot ${connected ? 'dot-ok' : 'dot-warn'}`;
    $('sseStatusText').textContent = connected ? '실시간 연결됨' : '연결 끊김 · 재연결 중...';
  }

  function connectSse() {
    const es = new EventSource(`${API_BASE}/events`);
    es.onopen = () => {
      setSseConnected(true);
    };
    es.onerror = () => {
      setSseConnected(false);
    };
    es.onmessage = (e) => handleSsePayload(e.data);
    for (const kind of SSE_EVENT_KINDS) {
      es.addEventListener(kind, (e) => handleSsePayload(e.data));
    }
  }

  // ---- Renderers ----

  function renderAll() {
    renderSessionHeader();
    renderSidebarSelected();
    renderEstates();
    renderPresets();
    renderFurniture();
    renderEnvironment();
    renderLogs();
  }

  function renderSessionHeader() {
    const sess = state.primarySession;
    if (!sess) {
      $('sessionPlayerText').textContent = '캐릭터 로그인 대기 중';
      $('sessionEstateText').textContent = '목록에서 하우스 미리 구성 가능';
      return;
    }

    const zoneText = ZONE_LOCATIONS[sess.zone_id] || `지역 #${sess.zone_id}`;
    $('sessionPlayerText').textContent = `${sess.character_name} · ${zoneText}`;
    if (sess.active_estate) {
      const ae = sess.active_estate;
      const loc = formatEstateLocation(ae);
      const size = formatSizeLabel(ae.plot_size, ae.is_apartment);
      $('sessionEstateText').textContent = `접속: ${loc} · ${size}`;
    } else {
      $('sessionEstateText').textContent = '현재 하우스 밖 (미선택)';
    }
  }

  function renderSidebarSelected() {
    const est = getSelectedEstate();
    $('navCountEstates').textContent = state.estates.length;
    $('navCountPresets').textContent = getAllPresets().length;
    $('navCountLogs').textContent = state.logs.length;

    const totalFurn = est ? (est.indoor_count + est.outdoor_count) : 0;
    const snapCount = state.selectedEstateDetail?.estate?.land_ident === est?.row?.land_ident
      ? (state.selectedEstateDetail.snapshots?.length ?? est.snapshot_count ?? 0)
      : (est?.snapshot_count ?? 0);
    $('navCountFurniture').textContent = totalFurn;
    $('navCountSnapshots').textContent = snapCount;

    const btnIds = ['sidebarBtnEnter', 'sidebarBtnExit', 'sidebarBtnReload', 'sidebarBtnUndo'];
    if (!est) {
      $('sidebarSizeBadge').textContent = '미선택';
      $('sidebarEstateTitle').textContent = '하우스를 선택하세요';
      $('sidebarEstateSub').textContent = '하우스 관리 목록에서 작업 대상을 선택해 주세요.';
      $('sidebarEstateStats').innerHTML = '';
      btnIds.forEach((id) => { $(id).disabled = true; });
      return;
    }

    const row = est.row;
    const loc = formatEstateLocation(row);
    const title = row.estate_name?.trim() ? `${row.estate_name} (${loc})` : loc;
    $('sidebarSizeBadge').textContent = formatSizeLabel(row.plot_size, row.is_apartment);
    $('sidebarEstateTitle').textContent = title;
    $('sidebarEstateSub').textContent = `${formatStyleName(est.pattern_name, '기본')} · 소유자 ${row.owner_name || 'Simulator'}`;
    $('sidebarEstateStats').innerHTML = `
      <span class="badge badge-muted">실내 ${est.indoor_count}</span>
      <span class="badge badge-muted">정원 ${est.outdoor_count}</span>
      <span class="badge badge-muted">스냅샷 ${snapCount}</span>
    `;
    btnIds.forEach((id) => { $(id).disabled = false; });
  }

  function getFilteredSortedEstates() {
    const query = $('estatesSearchInput').value.trim().toLowerCase();
    const dist = $('estatesDistrictFilter').value;
    const size = $('estatesSizeFilter').value;
    const sortBy = $('estatesSortSelect').value;

    const filtered = state.estates.filter((est) => {
      const r = est.row;
      if (state.favoritesOnly && !state.favorites[r.land_ident]) return false;
      if (dist !== 'all' && String(r.territory_type_id) !== dist) return false;
      if (size === 'apartment' && !r.is_apartment) return false;
      if (size !== 'all' && size !== 'apartment') {
        if (r.is_apartment || String(r.plot_size) !== size) return false;
      }
      if (!query) return true;
      const loc = formatEstateLocation(r).toLowerCase();
      const memo = (state.memos[r.land_ident] || '').toLowerCase();
      const hay = [
        r.estate_name,
        r.owner_name,
        est.pattern_name,
        loc,
        memo,
        String(r.land_ident),
      ].join(' ').toLowerCase();
      return hay.includes(query);
    });

    filtered.sort((a, b) => {
      const favDelta = Number(!!state.favorites[b.row.land_ident]) - Number(!!state.favorites[a.row.land_ident]);
      if (favDelta !== 0) return favDelta;

      if (sortBy === 'name') {
        const na = (a.row.estate_name || formatEstateLocation(a.row)).toLowerCase();
        const nb = (b.row.estate_name || formatEstateLocation(b.row)).toLowerCase();
        return na.localeCompare(nb, 'ko');
      }
      if (sortBy === 'furniture') {
        return (b.indoor_count + b.outdoor_count) - (a.indoor_count + a.outdoor_count);
      }
      if (sortBy === 'location') {
        return (
          a.row.territory_type_id - b.row.territory_type_id
          || a.row.ward_index - b.row.ward_index
          || Number(a.row.is_apartment) - Number(b.row.is_apartment)
          || (a.row.division * 30 + a.row.plot_index) - (b.row.division * 30 + b.row.plot_index)
          || a.row.room_number - b.row.room_number
        );
      }
      return (b.row.updated_at || 0) - (a.row.updated_at || 0);
    });

    return filtered;
  }

  function renderEstates() {
    const container = $('estatesContainer');
    const items = getFilteredSortedEstates();
    const onlineIdent = state.primarySession?.active_estate?.land_ident ?? null;

    $('btnViewCards').classList.toggle('active', state.viewMode === 'cards');
    $('btnViewTable').classList.toggle('active', state.viewMode === 'table');

    if (items.length === 0) {
      container.className = '';
      container.innerHTML = `<div class="empty-state">조건에 맞는 하우스가 없습니다. 상단의 [+ 새 하우스 생성 · 이동] 버튼으로 추가하거나 필터를 초기화하세요.</div>`;
      return;
    }

    if (state.viewMode === 'table') {
      container.className = 'estates-table-mode panel';
      container.innerHTML = `
        <div class="table-wrap">
          <table class="data-table">
            <thead>
              <tr>
                <th>★</th>
                <th>유형</th>
                <th>이름 / 위치</th>
                <th>내장재 양식</th>
                <th>소유자</th>
                <th>실내 / 정원</th>
                <th>작업</th>
              </tr>
            </thead>
            <tbody>
              ${items.map((est) => {
                const r = est.row;
                const isSel = r.land_ident === state.selectedLandIdent;
                const isOnline = r.land_ident === onlineIdent;
                const isFav = !!state.favorites[r.land_ident];
                const loc = formatEstateLocation(r);
                const memo = state.memos[r.land_ident];
                return `
                  <tr class="${isSel ? 'selected' : ''}" data-select-estate="${r.land_ident}">
                    <td><button class="fav-btn ${isFav ? 'active' : ''}" data-fav-estate="${r.land_ident}" type="button">${isFav ? '★' : '☆'}</button></td>
                    <td>
                      <span class="badge badge-accent">${esc(formatSizeLabel(r.plot_size, r.is_apartment))}</span>
                      ${isOnline ? '<span class="badge badge-success">접속 중</span>' : ''}
                    </td>
                    <td>
                      <strong>${esc(r.estate_name || loc)}</strong>
                      <div class="muted small-text">${esc(loc)}${memo ? ` · ${esc(memo)}` : ''}</div>
                    </td>
                    <td>${esc(formatStyleName(est.pattern_name, '기본'))}</td>
                    <td>${esc(r.owner_name || 'Simulator')}</td>
                    <td>${est.indoor_count} / ${est.outdoor_count}</td>
                    <td>
                      <div class="button-row">
                        <button class="btn btn-primary btn-xs" data-enter-estate="${r.land_ident}" type="button">실내</button>
                        <button class="btn btn-secondary btn-xs" data-exit-estate="${r.land_ident}" type="button">정문</button>
                        <button class="btn btn-danger btn-xs" data-delete-estate="${r.land_ident}" type="button">삭제</button>
                      </div>
                    </td>
                  </tr>
                `;
              }).join('')}
            </tbody>
          </table>
        </div>
      `;
      return;
    }

    container.className = 'estates-grid';
    container.innerHTML = items.map((est) => {
      const r = est.row;
      const isSel = r.land_ident === state.selectedLandIdent;
      const isOnline = r.land_ident === onlineIdent;
      const isFav = !!state.favorites[r.land_ident];
      const loc = formatEstateLocation(r);
      const memo = state.memos[r.land_ident];
      return `
        <article class="estate-card ${isSel ? 'selected' : ''} ${isOnline ? 'online' : ''}" data-select-estate="${r.land_ident}">
          <div class="estate-card-top">
            <div class="estate-card-badges">
              <span class="badge badge-accent">${esc(formatSizeLabel(r.plot_size, r.is_apartment))}</span>
              ${isOnline ? '<span class="badge badge-success">접속 중</span>' : ''}
              <span class="badge badge-muted">${esc(formatStyleName(est.pattern_name, '기본'))}</span>
            </div>
            <button class="fav-btn ${isFav ? 'active' : ''}" data-fav-estate="${r.land_ident}" type="button" title="즐겨찾기">${isFav ? '★' : '☆'}</button>
          </div>
          <div>
            <h3 class="estate-title">${esc(r.estate_name || loc)}</h3>
            <div class="estate-meta">${esc(loc)} · ${esc(r.owner_name || 'Simulator')}</div>
          </div>
          ${memo ? `<div class="estate-memo">${esc(memo)}</div>` : ''}
          <div class="estate-card-badges">
            <span class="badge badge-muted">실내 ${est.indoor_count}개</span>
            <span class="badge badge-muted">정원 ${est.outdoor_count}개</span>
            <span class="badge badge-muted">스냅샷 ${est.snapshot_count}개</span>
          </div>
          <div class="estate-actions">
            <button class="btn btn-primary btn-xs" data-enter-estate="${r.land_ident}" type="button">실내 입장</button>
            <button class="btn btn-secondary btn-xs" data-exit-estate="${r.land_ident}" type="button">정문 이동</button>
            <button class="btn btn-secondary btn-xs" data-reset-estate="${r.land_ident}" type="button">비우기</button>
            <button class="btn btn-danger btn-xs" data-delete-estate="${r.land_ident}" type="button">삭제</button>
          </div>
        </article>
      `;
    }).join('');
  }

  function renderPresets() {
    const targetSel = $('presetTargetSelect');
    targetSel.innerHTML = state.estates.length === 0
      ? '<option value="">보유 하우스 없음</option>'
      : state.estates.map((e) => {
          const r = e.row;
          const label = `${formatEstateLocation(r)} · ${formatSizeLabel(r.plot_size, r.is_apartment)}${r.estate_name ? ` (${r.estate_name})` : ''}`;
          return `<option value="${r.land_ident}" ${r.land_ident === state.selectedLandIdent ? 'selected' : ''}>${esc(label)}</option>`;
        }).join('');

    const search = $('presetsSearchInput').value.trim().toLowerCase();
    const sizeFilter = $('presetsSizeFilter').value;

    const list = getAllPresets().filter((p) => {
      if (sizeFilter === 'apartment' && !p.isApartment) return false;
      if (sizeFilter !== 'all' && sizeFilter !== 'apartment') {
        if (p.isApartment || String(p.normalizedPlotSize) !== sizeFilter) return false;
      }
      if (!search) return true;
      return `${p.title} ${p.styleKo} ${p.sourceKind}`.toLowerCase().includes(search);
    });

    const listEl = $('presetList');
    if (list.length === 0) {
      listEl.innerHTML = '<div class="empty-state">불러온 프리셋이 없습니다. 위 박스에 ReMakePlace JSON 파일을 드롭하세요.</div>';
    } else {
      listEl.innerHTML = list.map((p) => `
        <div class="preset-item ${p.id === state.selectedPresetId ? 'selected' : ''}" data-select-preset="${esc(p.id)}">
          <div class="preset-item-top">
            <div>
              <span class="badge ${p.sourceKind === '로컬' ? 'badge-success' : 'badge-accent'}">${esc(p.sourceKind)}</span>
              <span class="badge badge-muted">${esc(p.sizeBadge)}</span>
              <strong>${esc(p.title)}</strong>
            </div>
            <span class="muted small-text">실내 ${p.indoorCount} · 정원 ${p.outdoorCount}</span>
          </div>
          <div class="muted small-text">${esc(p.styleKo)}</div>
        </div>
      `).join('');
    }

    const preset = getSelectedPreset();
    const target = getSelectedEstate();
    const compat = evaluateCompat(preset, target);
    const banner = $('presetCompatBanner');
    banner.textContent = compat.text;
    banner.className = `compat-banner ${compat.status === 'match' ? 'match' : compat.status === 'warn' ? 'warn' : ''}`;
    $('btnPresetAutoMatch').classList.toggle('hidden', !compat.mismatch);

    if (!preset) {
      $('presetDetailTitle').textContent = '선택된 프리셋 없음';
      $('presetDetailBadge').textContent = '미선택';
      $('presetDetailCounts').textContent = '왼쪽 목록에서 프리셋을 선택하거나 JSON 파일을 드롭하세요.';
      $('presetDetailFixtures').innerHTML = '';
    } else if (preset.parseError) {
      $('presetDetailTitle').textContent = preset.title;
      $('presetDetailBadge').textContent = '파싱 오류';
      $('presetDetailCounts').textContent = preset.parseError;
      $('presetDetailFixtures').innerHTML = '';
    } else {
      $('presetDetailTitle').textContent = preset.title;
      $('presetDetailBadge').textContent = `${preset.sizeBadge} · ${preset.styleKo}`;
      const details = [];
      if (preset.indoorDyed > 0) details.push(`염색 ${preset.indoorDyed}`);
      if (preset.indoorMaterial > 0) details.push(`재질/그림 ${preset.indoorMaterial}`);
      const inSuffix = details.length ? ` (${details.join(', ')})` : '';
      const outSuffix = preset.outdoorDyed > 0 ? ` (염색 ${preset.outdoorDyed})` : '';
      $('presetDetailCounts').textContent =
        `실내 가구 ${preset.indoorCount}개${inSuffix} · 정원 가구 ${preset.outdoorCount}개${outSuffix}`;
      const inFix = preset.interiorFixtures.length ? preset.interiorFixtures.join(' / ') : '정보 없음';
      const outFix = preset.exteriorFixtures.length ? preset.exteriorFixtures.join(' / ') : '정보 없음';
      $('presetDetailFixtures').innerHTML = `
        <div><strong>실내 내장재:</strong> ${esc(inFix)}</div>
        <div><strong>외장재:</strong> ${esc(outFix)}</div>
      `;
    }
  }

  function getActiveFurnitureList() {
    if (!state.bridgeSnapshot) return [];
    return state.furnScope === 'indoor' ? (state.bridgeSnapshot.indoor || []) : (state.bridgeSnapshot.outdoor || []);
  }

  function syncSelectedFurnitureIntoDraft() {
    const list = getActiveFurnitureList();
    const item = list.find((f) => f.flat_slot === state.selectedFurnSlot) || null;
    if (!item) {
      state.selectedFurnSlot = null;
      state.transformDraft = null;
      state.transformOriginal = null;
      return;
    }
    const usePlotLocal = state.furnScope === 'outdoor' && $('tfPlotLocal').checked;
    const coords = {
      x: usePlotLocal ? item.local_x : item.pos_x,
      y: usePlotLocal ? item.local_y : item.pos_y,
      z: usePlotLocal ? item.local_z : item.pos_z,
      rotDeg: ((usePlotLocal ? item.local_rotation : item.rotation) * 180) / Math.PI,
    };
    state.transformDraft = { ...coords };
    state.transformOriginal = { ...coords };
  }

  function renderFurniture() {
    const snap = state.bridgeSnapshot;
    const indoorList = snap?.indoor || [];
    const outdoorList = snap?.outdoor || [];
    $('furnIndoorCount').textContent = indoorList.length;
    $('furnOutdoorCount').textContent = outdoorList.length;
    $('btnFurnScopeIndoor').classList.toggle('active', state.furnScope === 'indoor');
    $('btnFurnScopeOutdoor').classList.toggle('active', state.furnScope === 'outdoor');
    $('tfPlotLocalWrap').classList.toggle('hidden', state.furnScope !== 'outdoor');

    const search = $('furnSearchInput').value.trim().toLowerCase();
    const dyedOnly = $('furnDyedOnly').checked;
    const sortBy = $('furnSortSelect').value;

    const list = getActiveFurnitureList().filter((item) => {
      if (dyedOnly && !(item.stain > 0 || item.color_hex)) return false;
      if (!search) return true;
      return `${item.name} ${item.item_id} ${item.flat_slot}`.toLowerCase().includes(search);
    });

    list.sort((a, b) => {
      if (sortBy === 'name') return a.name.localeCompare(b.name, 'ko');
      if (sortBy === 'y') return b.pos_y - a.pos_y;
      return a.flat_slot - b.flat_slot;
    });

    const tbody = $('furnitureTableBody');
    if (!state.selectedLandIdent) {
      tbody.innerHTML = '<tr><td colspan="8" class="muted">먼저 하우스를 선택하세요.</td></tr>';
    } else if (list.length === 0) {
      tbody.innerHTML = '<tr><td colspan="8" class="muted">표시할 배치 가구가 없습니다.</td></tr>';
    } else {
      const usePlotLocal = state.furnScope === 'outdoor' && $('tfPlotLocal').checked;
      tbody.innerHTML = list.map((item) => {
        const x = usePlotLocal ? item.local_x : item.pos_x;
        const y = usePlotLocal ? item.local_y : item.pos_y;
        const z = usePlotLocal ? item.local_z : item.pos_z;
        const deg = (((usePlotLocal ? item.local_rotation : item.rotation) * 180) / Math.PI);
        const cleanHex = item.color_hex ? String(item.color_hex).replace(/^#/, '').slice(0, 6) : '';
        const swatch = /^[0-9a-fA-F]{6}$/.test(cleanHex)
          ? `<span class="swatch" style="background:#${cleanHex}"></span>`
          : '';
        return `
          <tr class="${item.flat_slot === state.selectedFurnSlot ? 'selected' : ''}" data-furn-slot="${item.flat_slot}">
            <td>${item.flat_slot}</td>
            <td><strong>${esc(item.name)}</strong></td>
            <td>#${item.item_id}</td>
            <td>${swatch}${item.stain > 0 ? item.stain : '-'}</td>
            <td>${x.toFixed(2)}</td>
            <td>${y.toFixed(2)}</td>
            <td>${z.toFixed(2)}</td>
            <td>${deg.toFixed(1)}°</td>
          </tr>
        `;
      }).join('');
    }

    const activeItem = getActiveFurnitureList().find((f) => f.flat_slot === state.selectedFurnSlot) || null;
    if (!activeItem || !state.transformDraft) {
      $('tfItemName').textContent = '가구를 선택하세요';
      $('tfItemMeta').textContent = '왼쪽 표에서 가구 행을 클릭하면 실시간 좌표·회전 조절기가 열립니다.';
      $('tfSlotBadge').textContent = '-';
      $('btnTfRevert').disabled = true;
      $('btnTfCommit').disabled = true;
      return;
    }

    $('tfItemName').textContent = activeItem.name;
    $('tfItemMeta').textContent = `Item #${activeItem.item_id} · Catalog #${activeItem.catalog_id}`;
    $('tfSlotBadge').textContent = `${state.furnScope === 'indoor' ? '실내' : '정원'} #${activeItem.flat_slot}`;
    $('tfPosX').value = state.transformDraft.x.toFixed(3);
    $('tfPosY').value = state.transformDraft.y.toFixed(3);
    $('tfPosZ').value = state.transformDraft.z.toFixed(3);
    $('tfRotDeg').value = state.transformDraft.rotDeg.toFixed(1);
    $('tfRotSlider').value = Math.max(-180, Math.min(180, state.transformDraft.rotDeg));
    $('btnTfRevert').disabled = false;
    $('btnTfCommit').disabled = false;
  }

  function renderEnvironment() {
    const detail = state.selectedEstateDetail;
    const estSummary = getSelectedEstate();
    if (!estSummary) {
      $('snapshotTimeline').innerHTML = '<div class="empty-state">하우스를 선택하면 스냅샷 목록이 표시됩니다.</div>';
      $('envFixturesContent').innerHTML = '';
      return;
    }

    const row = detail?.estate || estSummary.row;
    document.querySelectorAll('#envLightPills [data-light]').forEach((btn) => {
      btn.classList.toggle('active', Number(btn.dataset.light) === row.light_level);
    });
    $('envSsaoCheck').checked = !!row.enable_ssao;
    $('envVisitorsCheck').checked = (row.flags & 0x01) !== 0;

    $('envStyleSelect').disabled = !!row.is_apartment;
    $('btnEnvApplyStyle').disabled = !!row.is_apartment;
    $('envStyleSelect').value = normalizeStyleKey(estSummary.pattern_name);

    $('envNameInput').value = row.estate_name || '';
    $('envGreetingInput').value = row.greeting || '';
    $('envMemoInput').value = state.memos[row.land_ident] || '';
    $('envNameBytes').textContent = utf8ByteLength($('envNameInput').value);
    $('envGreetingBytes').textContent = utf8ByteLength($('envGreetingInput').value);

    const ext = detail?.exterior;
    const int = detail?.interior;
    if (ext && int) {
      $('envFixturesContent').innerHTML = `
        <div><strong>실내 양식 Row:</strong> ${int.renovation_row_id}</div>
        <div><strong>창문 / 문 ID:</strong> ${int.window_style} (염색 ${int.window_stain}) / ${int.door_style} (염색 ${int.door_stain})</div>
        <div><strong>1층 벽/바닥/조명:</strong> ${int.ground_walls} / ${int.ground_floor} / ${int.ground_chandelier}</div>
        <div><strong>2층 벽/바닥/조명:</strong> ${int.top_walls} / ${int.top_floor} / ${int.top_chandelier}</div>
        <div><strong>지하 벽/바닥/조명:</strong> ${int.cellar_walls} / ${int.cellar_floor} / ${int.cellar_chandelier}</div>
        <div><strong>외장 지붕/벽/창/문/울타리:</strong> ${ext.roof_id}/${ext.walls_id}/${ext.windows_id}/${ext.door_id}/${ext.fence_id}</div>
      `;
    } else {
      $('envFixturesContent').innerHTML = '<div class="muted">상세 데이터 로딩 중...</div>';
    }

    const snaps = detail?.snapshots || [];
    if (snaps.length === 0) {
      $('snapshotTimeline').innerHTML = '<div class="empty-state">저장된 스냅샷이 아직 없습니다. 프리셋 적용 시 자동 생성되며 수동 저장도 가능합니다.</div>';
    } else {
      $('snapshotTimeline').innerHTML = snaps.map((s) => `
        <div class="snapshot-item">
          <div>
            <strong>#${s.id} · ${esc(s.label)}</strong>
            <div class="muted small-text">${esc(formatTimestamp(s.created_at))}</div>
          </div>
          <button class="btn btn-secondary btn-sm" data-restore-snapshot="${s.id}" type="button">이 시점으로 복원</button>
        </div>
      `).join('');
    }
  }

  function renderLogs() {
    const q = $('logsSearchInput').value.trim().toLowerCase();
    const filtered = state.logs.filter((entry) => {
      if (state.logCategory !== 'all' && entry.category !== state.logCategory) return false;
      if (!q) return true;
      return `${entry.kind} ${entry.source} ${entry.message} ${entry.landIdent ?? ''}`.toLowerCase().includes(q);
    });

    $('navCountLogs').textContent = state.logs.length;
    const feed = $('logFeed');
    if (filtered.length === 0) {
      feed.innerHTML = '<div class="empty-state">표시할 실시간 로그가 없습니다.</div>';
      return;
    }

    feed.innerHTML = filtered.map((l) => `
      <div class="log-row">
        <span class="log-time">[${esc(l.time)}]</span>
        <span class="badge badge-muted">${esc(l.kind)}</span>
        ${l.source ? `<span class="badge badge-accent">${esc(l.source)}</span>` : ''}
        ${l.landIdent !== null ? `<span class="badge badge-muted">#${l.landIdent}</span>` : ''}
        <span>${esc(l.message)}</span>
      </div>
    `).join('');
  }

  // ---- Actions ----

  function buildCreateOrSwitchPayload(enterInterior) {
    const district = $('createDistrict').value;
    const divisionRaw = Number($('createDivision').value) || 0;
    const sizeVal = $('createSize').value;
    const isApartment = sizeVal === 'apartment';
    const plotSize = isApartment ? 0 : Number(sizeVal);
    const wardHuman = Math.max(1, Math.min(30, Number($('createWard').value) || 1));
    const plotOrRoomRaw = Number($('createPlotOrRoom').value);

    let division = divisionRaw;
    let plotIndex = null;
    let roomNumber = isApartment ? 1 : 0;

    if (plotOrRoomRaw > 0) {
      if (isApartment) {
        roomNumber = Math.max(1, Math.min(90, plotOrRoomRaw));
      } else {
        const zeroBased = Math.max(0, Math.min(59, plotOrRoomRaw - 1));
        division = zeroBased >= 30 ? 1 : 0;
        plotIndex = zeroBased % 30;
      }
    }

    const estateName = $('createEstateName').value.trim() || null;
    const ownerName = $('createOwnerName').value.trim() || null;

    return {
      district,
      ward_index: wardHuman - 1,
      division,
      plot_index: plotIndex,
      room_number: isApartment ? roomNumber : 0,
      is_apartment: isApartment,
      plot_size: (isApartment || plotIndex !== null) ? null : plotSize,
      estate_name: estateName,
      owner_name: ownerName || state.primarySession?.character_name || 'Simulator',
      owner_content_id: state.primarySession?.content_id || 1,
      enter_interior: enterInterior,
    };
  }

  async function ensureOnlyEstate() {
    try {
      const payload = buildCreateOrSwitchPayload(true);
      const row = await apiJson('/estates/ensure', {
        method: 'POST',
        body: JSON.stringify(payload),
      });
      toast(`하우스 준비 완료: ${formatEstateLocation(row)}`);
      $('createEstatePanel').classList.add('hidden');
      await refreshAll(row.land_ident);
    } catch (err) {
      toast(`하우스 생성 실패: ${err.message}`, true);
    }
  }

  async function switchWithCreateForm(enterInterior) {
    try {
      const payload = buildCreateOrSwitchPayload(enterInterior);
      const res = await apiJson('/session/switch', {
        method: 'POST',
        body: JSON.stringify(payload),
      });
      toast(`${enterInterior ? '실내 입장' : '정문 이동'} 완료: ${formatEstateLocation(res.estate)}`);
      $('createEstatePanel').classList.add('hidden');
      await refreshAll(res.estate.land_ident);
    } catch (err) {
      toast(`이동 실패: ${err.message}`, true);
    }
  }

  async function warpToEstate(landIdent, enterInterior) {
    try {
      const res = await apiJson('/session/switch', {
        method: 'POST',
        body: JSON.stringify({ land_ident: landIdent, enter_interior: enterInterior }),
      });
      toast(`${enterInterior ? '실내 입장' : '정문 이동'} 완료: ${formatEstateLocation(res.estate)}`);
      await refreshAll(res.estate.land_ident);
    } catch (err) {
      toast(`워프 실패: ${err.message}`, true);
    }
  }

  async function resetEstateScope(landIdent, scope) {
    const label = scope === 'interior' ? '실내' : scope === 'exterior' ? '정원' : '전체(실내+정원)';
    const ok = await askConfirm(
      '하우스 비우기 확인',
      `선택한 하우스의 ${label} 가구와 내·외장재를 초기화하시겠습니까? (복구용 스냅샷이 자동 생성됩니다)`
    );
    if (!ok) return;

    try {
      const res = await apiJson(`/estates/${landIdent}/reset`, {
        method: 'POST',
        body: JSON.stringify({ scope, reload_mode: 'auto', source: 'web_ui' }),
      });
      toast(`${label} 초기화 완료 (제거된 가구 ${res.removed_count}개)`);
      await refreshAll(landIdent);
    } catch (err) {
      toast(`초기화 실패: ${err.message}`, true);
    }
  }

  async function deleteEstate(landIdent) {
    const ok = await askConfirm(
      '하우스 영구 삭제',
      `하우스 #${landIdent} 레코드와 모든 가구·스냅샷을 DB에서 삭제합니다. 계속할까요?`
    );
    if (!ok) return;

    try {
      await apiJson(`/estates/${landIdent}`, { method: 'DELETE' });
      toast(`하우스 #${landIdent} 삭제 완료`);
      if (state.selectedLandIdent === landIdent) state.selectedLandIdent = null;
      await refreshAll();
    } catch (err) {
      toast(`삭제 실패: ${err.message}`, true);
    }
  }

  async function handlePickedPresetFiles(fileList) {
    const files = Array.from(fileList || []).filter((f) => f.name.toLowerCase().endsWith('.json'));
    if (files.length === 0) {
      toast('선택된 `.json` 파일이 없습니다.', true);
      return;
    }
    const parsed = await Promise.all(files.map(async (f) => {
      const buf = await f.arrayBuffer();
      const text = decodeBufferWithBom(buf);
      return parseLocalPresetFile(f.name, text, f.lastModified);
    }));

    for (const item of parsed) {
      const idx = state.localPresets.findIndex((p) => p.id === item.id);
      if (idx >= 0) state.localPresets[idx] = item;
      else state.localPresets.unshift(item);
    }
    state.selectedPresetId = parsed[0].id;
    renderPresets();
    renderSidebarSelected();
    toast(`${parsed.length}개 로컬 프리셋을 불러왔습니다.`);
  }

  function buildPresetRequestBody(preset) {
    const scope = $('presetScopeSelect').value;
    const reloadMode = $('presetReloadSelect').value;
    const preservePos = $('presetPreservePos').checked;
    return {
      ...(preset.rawJson !== null ? { raw_json: preset.rawJson } : { path: preset.serverPath }),
      scope,
      reload_mode: reloadMode,
      preserve_pos: preservePos,
      label: preset.title,
      source: 'web_ui',
    };
  }

  async function dispatchLiveMove({ commitDb }) {
    if (!state.selectedLandIdent || state.selectedFurnSlot === null || !state.transformDraft) return;
    const indoors = state.furnScope === 'indoor';
    const plotLocal = !indoors && $('tfPlotLocal').checked;
    const item = getActiveFurnitureList().find((f) => f.flat_slot === state.selectedFurnSlot);
    const d = state.transformDraft;
    const payload = {
      source: 'web_ui',
      indoors,
      plot_local: plotLocal,
      commit_db: commitDb,
      moves: [{
        flat_slot: state.selectedFurnSlot,
        item_id: item?.item_id ?? null,
        pos_x: Number(d.x),
        pos_y: Number(d.y),
        pos_z: Number(d.z),
        rotation: (Number(d.rotDeg) * Math.PI) / 180,
      }],
    };
    const res = await apiJson(`/estates/${state.selectedLandIdent}/live/move`, {
      method: 'POST',
      body: JSON.stringify(payload),
    });
    if (commitDb) {
      toast(`좌표 저장 및 RMP 동기화 완료 (rev ${res.revision})`);
      await refreshBridgeSnapshot();
    }
  }

  function scheduleLivePreview() {
    if (!$('tfAutoPreview').checked) return;
    clearTimeout(state.liveMoveTimer);
    state.liveMoveTimer = setTimeout(() => {
      dispatchLiveMove({ commitDb: false }).catch((err) => toast(`실시간 미리보기 오류: ${err.message}`, true));
    }, 80);
  }

  // ---- Event Wiring ----

  function bindEvents() {
    document.querySelectorAll('.nav-item').forEach((btn) => {
      btn.addEventListener('click', () => switchTab(btn.dataset.tab));
    });

    $('btnRefreshAll').addEventListener('click', () => refreshAll(state.selectedLandIdent));

    $('activeSessionPill').addEventListener('click', () => {
      const ident = state.primarySession?.active_estate?.land_ident;
      if (!ident) {
        toast('현재 게임에서 활성화된 하우스가 없습니다.', true);
        return;
      }
      selectEstate(ident);
      toast('현재 게임 위치 하우스를 선택했습니다.');
    });

    $('btnUnlockAll').addEventListener('click', async () => {
      try {
        const res = await apiJson('/session/unlock-all', { method: 'POST', body: '{}' });
        toast(`전체 해금 완료: ${res.unlocked_characters}명`);
      } catch (err) {
        toast(`해금 실패: ${err.message}`, true);
      }
    });

    document.querySelectorAll('[data-give-kit]').forEach((btn) => {
      btn.addEventListener('click', async () => {
        try {
          const kind = btn.dataset.giveKit;
          const res = await apiJson('/session/give-kit', {
            method: 'POST',
            body: JSON.stringify({ kind }),
          });
          toast(`샘플 키트(${kind}) 지급 완료 (온라인 ${res.online_sessions_notified}명)`);
        } catch (err) {
          toast(`키트 지급 실패: ${err.message}`, true);
        }
      });
    });

    // Sidebar pinned quick buttons
    $('sidebarBtnEnter').addEventListener('click', () => state.selectedLandIdent && warpToEstate(state.selectedLandIdent, true));
    $('sidebarBtnExit').addEventListener('click', () => state.selectedLandIdent && warpToEstate(state.selectedLandIdent, false));
    $('sidebarBtnReload').addEventListener('click', async () => {
      if (!state.selectedLandIdent) return;
      try {
        await apiJson(`/estates/${state.selectedLandIdent}/reload`, {
          method: 'POST',
          body: JSON.stringify({ mode: 'zone_reload', preserve_pos: true, force_enter: true }),
        });
        toast('강제 존 리로드를 전송했습니다.');
      } catch (err) {
        toast(`리로드 실패: ${err.message}`, true);
      }
    });
    $('sidebarBtnUndo').addEventListener('click', async () => {
      if (!state.selectedLandIdent) return;
      try {
        const res = await apiJson(`/estates/${state.selectedLandIdent}/undo`, {
          method: 'POST',
          body: JSON.stringify({ reload_mode: 'auto', preserve_pos: true, source: 'web_ui' }),
        });
        toast(`실행 취소 완료 (#${res.id})`);
        await refreshAll(state.selectedLandIdent);
      } catch (err) {
        toast(`실행 취소 실패: ${err.message}`, true);
      }
    });

    // Estates view
    $('btnToggleCreateEstate').addEventListener('click', () => $('createEstatePanel').classList.toggle('hidden'));
    $('btnCloseCreateEstate').addEventListener('click', () => $('createEstatePanel').classList.add('hidden'));
    $('btnCreateEnsureOnly').addEventListener('click', ensureOnlyEstate);
    $('btnCreateAndEnter').addEventListener('click', () => switchWithCreateForm(true));
    $('btnCreateAndOutside').addEventListener('click', () => switchWithCreateForm(false));

    ['estatesSearchInput', 'estatesDistrictFilter', 'estatesSizeFilter', 'estatesSortSelect'].forEach((id) => {
      $(id).addEventListener('input', renderEstates);
      $(id).addEventListener('change', renderEstates);
    });
    $('btnEstatesFavOnly').addEventListener('click', () => {
      state.favoritesOnly = !state.favoritesOnly;
      $('btnEstatesFavOnly').classList.toggle('btn-primary', state.favoritesOnly);
      renderEstates();
    });
    $('btnViewCards').addEventListener('click', () => { state.viewMode = 'cards'; savePrefs(); renderEstates(); });
    $('btnViewTable').addEventListener('click', () => { state.viewMode = 'table'; savePrefs(); renderEstates(); });

    $('estatesContainer').addEventListener('click', (ev) => {
      const favBtn = ev.target.closest('[data-fav-estate]');
      if (favBtn) {
        ev.stopPropagation();
        const id = Number(favBtn.dataset.favEstate);
        state.favorites[id] = !state.favorites[id];
        if (!state.favorites[id]) delete state.favorites[id];
        savePrefs();
        renderEstates();
        return;
      }
      const enterBtn = ev.target.closest('[data-enter-estate]');
      if (enterBtn) { ev.stopPropagation(); warpToEstate(Number(enterBtn.dataset.enterEstate), true); return; }
      const exitBtn = ev.target.closest('[data-exit-estate]');
      if (exitBtn) { ev.stopPropagation(); warpToEstate(Number(exitBtn.dataset.exitEstate), false); return; }
      const resetBtn = ev.target.closest('[data-reset-estate]');
      if (resetBtn) { ev.stopPropagation(); resetEstateScope(Number(resetBtn.dataset.resetEstate), 'all'); return; }
      const delBtn = ev.target.closest('[data-delete-estate]');
      if (delBtn) { ev.stopPropagation(); deleteEstate(Number(delBtn.dataset.deleteEstate)); return; }

      const rowOrCard = ev.target.closest('[data-select-estate]');
      if (rowOrCard) {
        selectEstate(Number(rowOrCard.dataset.selectEstate));
      }
    });

    // Presets view
    $('presetTargetSelect').addEventListener('change', (ev) => {
      const val = Number(ev.target.value);
      if (val) selectEstate(val);
    });
    $('btnPresetSyncActive').addEventListener('click', () => {
      const ident = state.primarySession?.active_estate?.land_ident;
      if (!ident) {
        toast('현재 게임에서 활성화된 하우스가 없습니다.', true);
        return;
      }
      selectEstate(ident);
    });
    $('btnPresetAutoMatch').addEventListener('click', () => {
      const preset = getSelectedPreset();
      if (!preset || preset.normalizedPlotSize === null) return;
      const match = state.estates.find((e) =>
        e.row.is_apartment === preset.isApartment && (preset.isApartment || e.row.plot_size === preset.normalizedPlotSize)
      );
      if (!match) {
        toast(`[${preset.sizeBadge}] 크기의 하우스가 없습니다. 먼저 생성하세요.`, true);
        return;
      }
      selectEstate(match.row.land_ident);
    });

    $('btnPickPresetFiles').addEventListener('click', () => $('presetFilesInput').click());
    $('btnPickPresetFolder').addEventListener('click', () => $('presetFolderInput').click());
    $('presetFilesInput').addEventListener('change', (ev) => { handlePickedPresetFiles(ev.target.files); ev.target.value = ''; });
    $('presetFolderInput').addEventListener('change', (ev) => { handlePickedPresetFiles(ev.target.files); ev.target.value = ''; });
    $('btnRefreshServerPresets').addEventListener('click', () => refreshAll(state.selectedLandIdent));

    const dz = $('presetDropZone');
    dz.addEventListener('dragover', (ev) => { ev.preventDefault(); dz.classList.add('dragover'); });
    dz.addEventListener('dragleave', () => dz.classList.remove('dragover'));
    dz.addEventListener('drop', (ev) => {
      ev.preventDefault();
      dz.classList.remove('dragover');
      handlePickedPresetFiles(ev.dataTransfer?.files);
    });

    ['presetsSearchInput', 'presetsSizeFilter'].forEach((id) => {
      $(id).addEventListener('input', renderPresets);
      $(id).addEventListener('change', renderPresets);
    });

    $('presetList').addEventListener('click', (ev) => {
      const item = ev.target.closest('[data-select-preset]');
      if (!item) return;
      state.selectedPresetId = item.dataset.selectPreset;
      $('presetPreviewReport').classList.add('hidden');
      renderPresets();
    });

    $('btnPreviewPreset').addEventListener('click', async () => {
      const preset = getSelectedPreset();
      const target = getSelectedEstate();
      if (!preset || !target) { toast('대상 하우스와 프리셋을 선택하세요.', true); return; }
      try {
        const report = await apiJson('/presets/preview', {
          method: 'POST',
          body: JSON.stringify({
            ...(preset.rawJson !== null ? { raw_json: preset.rawJson } : { path: preset.serverPath }),
            scope: $('presetScopeSelect').value,
            land_ident: target.row.land_ident,
          }),
        });
        const box = $('presetPreviewReport');
        box.classList.remove('hidden');
        const lines = [
          `프리셋 크기: ${report.house_size || '미지정'}`,
          `실내: ${report.interior_resolved_count}/${report.interior_input_count} (잘림 ${report.interior_truncated_count}, 내장 ${report.interior_fixtures_matched})`,
          `정원: ${report.exterior_resolved_count}/${report.exterior_input_count} (잘림 ${report.exterior_truncated_count}, 외장 ${report.exterior_fixtures_matched})`,
        ];
        if (report.warnings?.length) lines.push('경고:\n- ' + report.warnings.join('\n- '));
        if (report.interior_unresolved_items?.length) lines.push('미매칭 실내: ' + report.interior_unresolved_items.slice(0, 8).join(', '));
        if (report.exterior_unresolved_items?.length) lines.push('미매칭 정원: ' + report.exterior_unresolved_items.slice(0, 8).join(', '));
        box.textContent = lines.join('\n');
      } catch (err) {
        toast(`검증 실패: ${err.message}`, true);
      }
    });

    $('btnApplyPreset').addEventListener('click', async () => {
      const preset = getSelectedPreset();
      const target = getSelectedEstate();
      if (!preset || !target) { toast('대상 하우스와 프리셋을 선택하세요.', true); return; }
      const compat = evaluateCompat(preset, target);
      if (compat.mismatch) {
        const proceed = await askConfirm('크기 불일치 경고', `${compat.text}\n그래도 적용할까요?`);
        if (!proceed) return;
      }
      try {
        const res = await apiJson(`/estates/${target.row.land_ident}/preset/apply`, {
          method: 'POST',
          body: JSON.stringify(buildPresetRequestBody(preset)),
        });
        toast(`적용 완료: 실내 ${res.preview.interior_resolved_count}, 정원 ${res.preview.exterior_resolved_count}`);
        await refreshAll(target.row.land_ident);
      } catch (err) {
        toast(`적용 실패: ${err.message}`, true);
      }
    });

    $('btnExportDownload').addEventListener('click', async () => {
      if (!state.selectedLandIdent) { toast('하우스를 먼저 선택하세요.', true); return; }
      try {
        const scope = $('exportScopeSelect').value;
        const layout = await apiJson(`/estates/${state.selectedLandIdent}/export/remakeplace?scope=${encodeURIComponent(scope)}`);
        const blob = new Blob([JSON.stringify(layout, null, 2)], { type: 'application/json' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        const rawName = ($('exportFilenameInput').value.trim() || `estate_${state.selectedLandIdent}_${scope}`).replace(/\.json$/i, '');
        a.href = url;
        a.download = `${rawName}.json`;
        a.click();
        URL.revokeObjectURL(url);
        toast('ReMakePlace JSON 파일을 다운로드했습니다.');
      } catch (err) {
        toast(`내보내기 실패: ${err.message}`, true);
      }
    });

    $('btnExportSaveServer').addEventListener('click', async () => {
      if (!state.selectedLandIdent) { toast('하우스를 먼저 선택하세요.', true); return; }
      try {
        const res = await apiJson(`/estates/${state.selectedLandIdent}/export/remakeplace`, {
          method: 'POST',
          body: JSON.stringify({
            scope: $('exportScopeSelect').value,
            save_to_disk: true,
            filename: $('exportFilenameInput').value.trim() || null,
          }),
        });
        toast(`서버 저장 완료: ${res.saved_path}`);
      } catch (err) {
        toast(`서버 저장 실패: ${err.message}`, true);
      }
    });

    // Furniture Inspector & Transform Editor
    $('btnRefreshFurniture').addEventListener('click', refreshBridgeSnapshot);
    $('btnFurnScopeIndoor').addEventListener('click', () => {
      state.furnScope = 'indoor';
      state.selectedFurnSlot = null;
      syncSelectedFurnitureIntoDraft();
      renderFurniture();
    });
    $('btnFurnScopeOutdoor').addEventListener('click', () => {
      state.furnScope = 'outdoor';
      state.selectedFurnSlot = null;
      syncSelectedFurnitureIntoDraft();
      renderFurniture();
    });
    $('tfPlotLocal').addEventListener('change', () => {
      syncSelectedFurnitureIntoDraft();
      renderFurniture();
    });
    ['furnSearchInput', 'furnDyedOnly', 'furnSortSelect'].forEach((id) => {
      $(id).addEventListener('input', renderFurniture);
      $(id).addEventListener('change', renderFurniture);
    });

    $('furnitureTableBody').addEventListener('click', (ev) => {
      const tr = ev.target.closest('[data-furn-slot]');
      if (!tr) return;
      state.selectedFurnSlot = Number(tr.dataset.furnSlot);
      syncSelectedFurnitureIntoDraft();
      renderFurniture();
    });

    const onAxisEdit = (axis, value) => {
      if (!state.transformDraft || Number.isNaN(value)) return;
      if (axis === 'x') state.transformDraft.x = value;
      if (axis === 'y') state.transformDraft.y = value;
      if (axis === 'z') state.transformDraft.z = value;
      if (axis === 'rot') state.transformDraft.rotDeg = value;
      if (axis === 'rot') $('tfRotSlider').value = Math.max(-180, Math.min(180, value));
      scheduleLivePreview();
    };

    $('tfPosX').addEventListener('input', (e) => onAxisEdit('x', parseFloat(e.target.value)));
    $('tfPosY').addEventListener('input', (e) => onAxisEdit('y', parseFloat(e.target.value)));
    $('tfPosZ').addEventListener('input', (e) => onAxisEdit('z', parseFloat(e.target.value)));
    $('tfRotDeg').addEventListener('input', (e) => onAxisEdit('rot', parseFloat(e.target.value)));
    $('tfRotSlider').addEventListener('input', (e) => {
      if (!state.transformDraft) return;
      const v = parseFloat(e.target.value);
      state.transformDraft.rotDeg = v;
      $('tfRotDeg').value = v.toFixed(1);
      scheduleLivePreview();
    });

    document.querySelectorAll('.btn-nudge').forEach((btn) => {
      btn.addEventListener('click', () => {
        if (!state.transformDraft) return;
        const axis = btn.dataset.axis;
        const delta = parseFloat(btn.dataset.delta);
        if (axis === 'x') state.transformDraft.x = Number((state.transformDraft.x + delta).toFixed(4));
        if (axis === 'y') state.transformDraft.y = Number((state.transformDraft.y + delta).toFixed(4));
        if (axis === 'z') state.transformDraft.z = Number((state.transformDraft.z + delta).toFixed(4));
        if (axis === 'rot') state.transformDraft.rotDeg = Number((state.transformDraft.rotDeg + delta).toFixed(2));
        renderFurniture();
        scheduleLivePreview();
      });
    });

    $('btnTfRevert').addEventListener('click', () => {
      if (!state.transformOriginal) return;
      state.transformDraft = { ...state.transformOriginal };
      renderFurniture();
      dispatchLiveMove({ commitDb: false }).catch((err) => toast(`되돌리기 오류: ${err.message}`, true));
    });
    $('btnTfCommit').addEventListener('click', () => {
      dispatchLiveMove({ commitDb: true }).catch((err) => toast(`저장 실패: ${err.message}`, true));
    });

    // Environment & Snapshots
    document.querySelectorAll('#envLightPills [data-light]').forEach((btn) => {
      btn.addEventListener('click', () => {
        document.querySelectorAll('#envLightPills [data-light]').forEach((b) => b.classList.remove('active'));
        btn.classList.add('active');
      });
    });

    $('envNameInput').addEventListener('input', (e) => { $('envNameBytes').textContent = utf8ByteLength(e.target.value); });
    $('envGreetingInput').addEventListener('input', (e) => { $('envGreetingBytes').textContent = utf8ByteLength(e.target.value); });

    $('btnEnvSaveLight').addEventListener('click', async () => {
      if (!state.selectedLandIdent) return;
      const activeBtn = document.querySelector('#envLightPills .seg-btn.active');
      const lightLevel = activeBtn ? Number(activeBtn.dataset.light) : 3;
      try {
        await apiJson(`/estates/${state.selectedLandIdent}`, {
          method: 'PATCH',
          body: JSON.stringify({
            light_level: lightLevel,
            enable_ssao: $('envSsaoCheck').checked,
            visitors_allowed: $('envVisitorsCheck').checked,
            reload_mode: $('envReloadSelect').value,
            source: 'web_ui',
          }),
        });
        toast(`조명 (${lightLevel}) 및 옵션 반영 완료`);
        await refreshEstateDetail({ silent: true });
      } catch (err) {
        toast(`설정 반영 실패: ${err.message}`, true);
      }
    });

    $('btnEnvApplyStyle').addEventListener('click', async () => {
      if (!state.selectedLandIdent) return;
      try {
        await apiJson(`/estates/${state.selectedLandIdent}`, {
          method: 'PATCH',
          body: JSON.stringify({
            interior_style: $('envStyleSelect').value,
            reload_mode: 'zone_reload',
            source: 'web_ui',
          }),
        });
        toast('실내 내장재 양식을 적용했습니다.');
        await refreshAll(state.selectedLandIdent);
      } catch (err) {
        toast(`양식 적용 실패: ${err.message}`, true);
      }
    });

    $('btnEnvSaveMeta').addEventListener('click', async () => {
      if (!state.selectedLandIdent) return;
      try {
        await apiJson(`/estates/${state.selectedLandIdent}`, {
          method: 'PATCH',
          body: JSON.stringify({
            estate_name: $('envNameInput').value.trim(),
            greeting: $('envGreetingInput').value.trim(),
            reload_mode: 'live_packets',
            source: 'web_ui',
          }),
        });
        state.memos[state.selectedLandIdent] = $('envMemoInput').value.trim();
        savePrefs();
        toast('이름 및 인사말 저장 완료');
        await refreshAll(state.selectedLandIdent);
      } catch (err) {
        toast(`저장 실패: ${err.message}`, true);
      }
    });

    $('btnEnvSaveMemo').addEventListener('click', () => {
      if (!state.selectedLandIdent) return;
      const memo = $('envMemoInput').value.trim();
      if (memo) state.memos[state.selectedLandIdent] = memo;
      else delete state.memos[state.selectedLandIdent];
      savePrefs();
      renderEstates();
      toast('로컬 메모를 저장했습니다.');
    });

    $('btnResetIndoor').addEventListener('click', () => state.selectedLandIdent && resetEstateScope(state.selectedLandIdent, 'interior'));
    $('btnResetExterior').addEventListener('click', () => state.selectedLandIdent && resetEstateScope(state.selectedLandIdent, 'exterior'));
    $('btnResetAll').addEventListener('click', () => state.selectedLandIdent && resetEstateScope(state.selectedLandIdent, 'all'));
    $('btnDeleteEstate').addEventListener('click', () => state.selectedLandIdent && deleteEstate(state.selectedLandIdent));
    $('btnEnvForceReload').addEventListener('click', () => $('sidebarBtnReload').click());

    $('btnSnapshotCreate').addEventListener('click', async () => {
      if (!state.selectedLandIdent) return;
      try {
        const res = await apiJson(`/estates/${state.selectedLandIdent}/snapshots`, {
          method: 'POST',
          body: JSON.stringify({ label: $('snapshotLabelInput').value.trim() || null }),
        });
        $('snapshotLabelInput').value = '';
        toast(`스냅샷 #${res.id} 저장 완료`);
        await refreshEstateDetail({ silent: true });
      } catch (err) {
        toast(`스냅샷 저장 실패: ${err.message}`, true);
      }
    });

    $('btnSnapshotUndo').addEventListener('click', () => $('sidebarBtnUndo').click());

    $('snapshotTimeline').addEventListener('click', async (ev) => {
      const btn = ev.target.closest('[data-restore-snapshot]');
      if (!btn || !state.selectedLandIdent) return;
      const snapId = Number(btn.dataset.restoreSnapshot);
      const ok = await askConfirm('스냅샷 복원', `스냅샷 #${snapId} 시점으로 가구·외장·내장을 복원할까요?`);
      if (!ok) return;
      try {
        await apiJson(`/estates/${state.selectedLandIdent}/snapshots/${snapId}/restore`, {
          method: 'POST',
          body: JSON.stringify({ reload_mode: 'auto', preserve_pos: true, source: 'web_ui' }),
        });
        toast(`스냅샷 #${snapId} 상태로 복원했습니다.`);
        await refreshAll(state.selectedLandIdent);
      } catch (err) {
        toast(`복원 실패: ${err.message}`, true);
      }
    });

    // Logs
    $('logCategoryFilter').addEventListener('click', (ev) => {
      const btn = ev.target.closest('[data-log-cat]');
      if (!btn) return;
      state.logCategory = btn.dataset.logCat;
      document.querySelectorAll('#logCategoryFilter [data-log-cat]').forEach((b) => {
        b.classList.toggle('active', b === btn);
      });
      renderLogs();
    });
    $('logsSearchInput').addEventListener('input', renderLogs);
    $('btnClearLogs').addEventListener('click', () => { state.logs = []; renderLogs(); });
    $('btnCopyLogs').addEventListener('click', async () => {
      const text = state.logs.map((l) => `[${l.time}] [${l.kind}] ${l.message}`).join('\n');
      try {
        await navigator.clipboard.writeText(text);
        toast('로그를 클립보드에 복사했습니다.');
      } catch {
        toast('클립보드 복사에 실패했습니다.', true);
      }
    });
  }

  // ---- Init ----

  loadPrefs();
  bindEvents();
  switchTab(state.activeTab);
  connectSse();
  refreshAll();
})();
