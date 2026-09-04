# training/ — OOV neural fallback (milestone M7, not started)

The rule engine + dictionary handle ~90–95% of real typing. This directory is for
the **optional** last layer: a small character-level transformer that transliterates
Latin → Devanagari for words the dictionary doesn't know (names, rare words,
creative spellings). It is exported to **int8 ONNX**, loaded lazily by the engine,
and off by default.

## Plan

1. **Data** — Nepali romanization pairs:
   - Dakshina dataset, `ne` split (word lexicon + romanized sentences)
   - Aksharantar, `nep` split
   - mined pairs from Nepali Wikipedia titles + their common romanizations
2. **Tokenizer** — character-level, vocab ~150 (Latin letters + Devanagari
   code points + specials).
3. **Model** — 4+4 layer transformer, `d_model=256`, ~6M params. Train with
   label smoothing 0.1, ~30 epochs. Beam search (size 5) at inference → top-k
   candidates.
4. **Eval** — top-1 / top-5 accuracy and CER on a held-out Nepali word list.
   Gate: only ship if it beats "rule engine alone" on OOV words by a clear margin.
5. **Export** — `optimum-cli export onnx ...` (encoder + decoder + past-kv),
   then `onnxruntime.quantization.quantize_dynamic` → int8.
6. **Integrate** — a `Ranker` in `xlit-core` behind a `model` feature flag that
   loads the `.onnx` via `ort`, runs only when no dictionary candidate clears a
   confidence threshold, and unloads after idle.

## Alternative

If ONNX beam-search plumbing gets in the way, use **CTranslate2** instead — it
converts HF/Marian/fairseq checkpoints directly and has native int8 + beam search.

Nothing in here is on the critical path for M2–M6.
