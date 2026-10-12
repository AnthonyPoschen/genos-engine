#!/usr/bin/env python3
"""S78S lighting dump decoder.

dump.py DIR N   -> splits DIR/f{N}_*.bin (and f{N+1}) into one file per component
and channel under DIR/out/, plus N vs N+1 diff heatmaps:

  sc_{fine,coarse}_f{F}.csv        every live surface-cache / world-cache slot:
                                   position, normal, size, value, sets, last use,
                                   sun share, estimator variance, TDL direct
  sc_{fine,coarse}_{chan}_f{F}.png slot atlases (1024 x 1024, slot = y*1024+x),
                                   chan in value, sets, variance, tdl_direct,
                                   last_used_age, dirty_stamp_age
  sc_{fine,coarse}_flip_xz.png     top-down (x, z) heatmap of slots whose value
                                   moved > 5 % from N to N+1 (log count)
  sc_{fine,coarse}_flips.csv       those slots with both frames' fields
  sc_{fine,coarse}_delta_atlas.png per-slot |delta| / value atlas (N -> N+1)
  probe_irradiance_f{F}.png        probes' filtered SH DC (rgb, tone-mapped)
  probe_changed_mask.png           probes with any kept-hit light changed N->N+1
  probe_changed_count.png          how many of their kept hits changed
  probe_own_strata_f{F}.png        strata read from the probe itself (8 = all)
  kept_sources_f{F}.csv            per probe: own strata, fills, kept-hit misses
  pixel_direct_f{F}.png            per-pixel direct light (G-buffer w)
  pixel_direct_delta.png           its N -> N+1 change heatmap
"""
import json, os, sys
import numpy as np
from PIL import Image

S = 1048576
LW = np.array([0.2126, 0.7152, 0.0722], np.float32)
EST_AT, SUN_AT, TDL_AT, DIRTY_AT = 4 * S, 7 * S, 14 * S, 21 * S
B, CR, HEAD, CHANGES = 98304, 16, 5, 64


def rgb9e5(u):
    u = u.astype(np.uint32)
    e = ((u >> 27) & 31).astype(np.int32) - 15 - 9
    sc = np.power(2.0, e).astype(np.float32)
    return np.stack([(u & 511) * sc, ((u >> 9) & 511) * sc, ((u >> 18) & 511) * sc], -1)


def tone(x, hi=None):
    hi = hi or max(np.percentile(x[np.isfinite(x)], 99.5), 1e-6)
    return (np.clip(x / hi, 0, 1) ** (1 / 2.2) * 255).astype(np.uint8)


def heat(x, hi=None):
    x = np.nan_to_num(x)
    hi = hi or max(np.percentile(x, 99.5), 1e-6)
    t = np.clip(x / hi, 0, 1)
    rgb = np.stack([np.clip(3 * t, 0, 1), np.clip(3 * t - 1, 0, 1), np.clip(3 * t - 2, 0, 1)], -1)
    return (rgb * 255).astype(np.uint8)


def atlas(vals, mask, path, hm=False):
    a = np.zeros(S, np.float32)
    a[mask] = vals[mask]
    img = heat(a.reshape(1024, 1024)) if hm else tone(a.reshape(1024, 1024))
    Image.fromarray(img).save(path)


def load(d, f):
    lay = json.load(open(f'{d}/f{f}_layout.json'))
    c = np.fromfile(f'{d}/f{f}_cache.bin', np.float32).reshape(-1, 4)
    cu = c.view(np.uint32)
    k = np.fromfile(f'{d}/f{f}_keys.bin', np.uint32)
    w = np.fromfile(f'{d}/f{f}_work.bin', np.float32).reshape(-1, 4)
    g = np.fromfile(f'{d}/f{f}_gbuf.bin', np.uint32).reshape(-1, 4)
    i = np.arange(S)
    sc = dict(
        live=k[:S] != 0,
        pos=c[3 * i, :3], used=cu[3 * i, 3], nrm=c[3 * i + 1, :3], size=c[3 * i + 1, 3],
        val=c[3 * i + 2, :3], sets=c[3 * i + 2, 3], sunf=c[SUN_AT + i, 0],
        var=c[EST_AT + 2 * i, 3], tdl=rgb9e5(cu[TDL_AT + i, 0]), tdl_frame=cu[TDL_AT + i, 3],
        dirty=cu[DIRTY_AT + 2 * i],
    )
    return lay, sc, w, g


def work_layout(lay):
    P = lay['probes_x'] * lay['probes_y']
    R, St = lay['rays'], lay['strata']
    K = P * R * St
    shf = 2 * P + B * CR
    strat = shf + 7 * P
    kept = strat + 2 * P * St
    kept_light = kept + K
    change = kept_light + K // 4
    view = change + HEAD + CHANGES
    return dict(P=P, R=R, St=St, K=K, shf=shf, kept=kept, kept_light=kept_light, change=change, view=view)


def main():
    d, n = sys.argv[1], int(sys.argv[2])
    o = f'{d}/out'
    os.makedirs(o, exist_ok=True)
    frames = [n, n + 1]
    data = {f: load(d, f) for f in frames}
    lay = data[n][0]
    wl = work_layout(lay)
    P, R, St, K = wl['P'], wl['R'], wl['St'], wl['K']
    px, py = lay['probes_x'], lay['probes_y']
    W, H = lay['width'], lay['height']
    for f in frames:
        _, sc, w, g = data[f]
        lum = sc['val'] @ LW
        for cls, m in [('fine', sc['live'] & (sc['size'] <= 0.13)), ('coarse', sc['live'] & (sc['size'] > 0.13))]:
            idx = np.nonzero(m)[0]
            cols = np.column_stack([idx, sc['pos'][idx], sc['nrm'][idx], sc['size'][idx], sc['val'][idx], lum[idx],
                                    sc['sets'][idx], sc['used'][idx], sc['sunf'][idx], sc['var'][idx],
                                    sc['tdl'][idx], sc['tdl_frame'][idx]])
            np.savetxt(f'{o}/sc_{cls}_f{f}.csv', cols, delimiter=',', fmt='%.6g',
                       header='slot,x,y,z,nx,ny,nz,size,r,g,b,lum,sets,last_used,sunfrac,est_var,tdl_r,tdl_g,tdl_b,tdl_frame', comments='')
            atlas(lum, m, f'{o}/sc_{cls}_value_f{f}.png')
            atlas(sc['sets'], m, f'{o}/sc_{cls}_sets_f{f}.png')
            atlas(sc['var'], m, f'{o}/sc_{cls}_variance_f{f}.png', hm=True)
            atlas(sc['tdl'] @ LW, m, f'{o}/sc_{cls}_tdl_direct_f{f}.png')
            atlas(np.where(sc['used'] > 0, f - sc['used'].astype(np.float32), 0), m, f'{o}/sc_{cls}_last_used_age_f{f}.png', hm=True)
            atlas(np.where(sc['dirty'][:, 0] > 0, f - sc['dirty'][:, 0].astype(np.float32), 0), m, f'{o}/sc_{cls}_dirty_stamp_age_f{f}.png', hm=True)
        sh = w[wl['shf'] + 7 * np.arange(P)]
        Image.fromarray(tone(sh[:, :3].reshape(py, px, 3))).resize((px * 8, py * 8), Image.NEAREST).save(f'{o}/probe_irradiance_f{f}.png')
        views = w[wl['view'] + np.arange(2 * 2 * P)].view(np.uint32)  # both blocks; the current one is unknown here: take both
        own = np.zeros(P)
        fills = np.zeros(P)
        for blk in (0, 1):
            v = views[blk * 2 * P:(blk + 1) * 2 * P].reshape(P, 8)
            o_ = (v == (np.arange(P)[:, None] + 1)).sum(1)
            if o_.mean() > own.mean():
                own, fills = o_, (v == 0).sum(1)
        Image.fromarray(heat(own.reshape(py, px), 8)).resize((px * 8, py * 8), Image.NEAREST).save(f'{o}/probe_own_strata_f{f}.png')
        rec = w[wl['kept']:wl['kept'] + K]
        miss = (rec[:, 0] < 0).reshape(P, St * R).sum(1)
        np.savetxt(f'{o}/kept_sources_f{f}.csv', np.column_stack([np.arange(P), own, fills, miss]), delimiter=',', fmt='%d',
                   header='probe,own_strata,fill_strata,kept_misses', comments='')
        pd = rgb9e5(g[:W * H, 3]) @ LW
        Image.fromarray(tone(pd.reshape(H, W))).save(f'{o}/pixel_direct_f{f}.png')
    # N -> N+1 diffs
    _, a, wa, ga = data[n]
    _, b, wb, gb = data[n + 1]
    la, lb = a['val'] @ LW, b['val'] @ LW
    rel = np.abs(lb - la) / np.maximum(np.maximum(la, lb), 1e-4)
    both = a['live'] & b['live'] & (a['pos'] == b['pos']).all(1)
    for cls, m in [('fine', both & (a['size'] <= 0.13)), ('coarse', both & (a['size'] > 0.13))]:
        atlas(rel, m, f'{o}/sc_{cls}_delta_atlas.png', hm=True)
        fl = m & (rel > 0.05)
        idx = np.nonzero(fl)[0]
        print(f'{cls}: live {m.sum()} changed {(m & (rel > 0)).sum()} >5% {fl.sum()}; '
              f'sets of flips: median {np.median(a["sets"][idx]) if len(idx) else 0:.2f}; '
              f'sets delta {np.median(b["sets"][idx] - a["sets"][idx]) if len(idx) else 0:.2f}')
        cols = np.column_stack([idx, a['pos'][idx], a['size'][idx], la[idx], lb[idx], rel[idx], a['sets'][idx], b['sets'][idx],
                                a['var'][idx], b['var'][idx], a['used'][idx], b['used'][idx]])
        np.savetxt(f'{o}/sc_{cls}_flips.csv', cols, delimiter=',', fmt='%.6g',
                   header='slot,x,y,z,size,lum_n,lum_n1,rel,sets_n,sets_n1,var_n,var_n1,used_n,used_n1', comments='')
        if len(idx):
            p = a['pos'][idx]
            hx, xe, ze = np.histogram2d(p[:, 0], p[:, 2], bins=256)
            Image.fromarray(heat(np.log1p(hx).T)).resize((768, 768), Image.NEAREST).save(f'{o}/sc_{cls}_flip_xz.png')
    kl = wa[wl['kept_light']:wl['kept_light'] + K // 4].view(np.uint32).reshape(-1)
    kl2 = wb[wl['kept_light']:wl['kept_light'] + K // 4].view(np.uint32).reshape(-1)
    ch = (kl != kl2).reshape(P, St * R).sum(1)
    Image.fromarray(heat((ch > 0).astype(float).reshape(py, px), 1)).resize((px * 8, py * 8), Image.NEAREST).save(f'{o}/probe_changed_mask.png')
    Image.fromarray(heat(ch.reshape(py, px).astype(float))).resize((px * 8, py * 8), Image.NEAREST).save(f'{o}/probe_changed_count.png')
    print(f'probes {P}: changed {(ch > 0).sum()}, kept-light words changed {(kl != kl2).sum()}')
    # Which kept hits changed: stratum (vs this frame's) and whether k is the held check ray.
    h = np.nonzero(kl != kl2)[0]
    pr, s_, k_ = h // (St * R), (h // R) % St, h % R
    def lhash(v):
        v = v.astype(np.uint64) & 0xffffffff
        v ^= v >> 16; v = (v * 0x7feb352d) & 0xffffffff; v ^= v >> 15; v = (v * 0x846ca68b) & 0xffffffff; v ^= v >> 16
        return v
    for fr in (n, n + 1):
        ck = lhash(((pr.astype(np.uint64) * 0x9e3779b9) ^ (fr * 0x85ebca6b)) & 0xffffffff) % R
        print(f'  frame {fr}: changed in stratum {fr % St}: {(s_ == fr % St).mean():.2f}; k is the held check ray: {(k_ == ck).mean():.2f}')
    print('  strata of changed hits:', np.bincount(s_, minlength=St))
    pa, pb = rgb9e5(ga[:W * H, 3]) @ LW, rgb9e5(gb[:W * H, 3]) @ LW
    Image.fromarray(heat(np.abs(pb - pa).reshape(H, W))).save(f'{o}/pixel_direct_delta.png')


if __name__ == '__main__':
    main()
