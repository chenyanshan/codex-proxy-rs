/*
 * 基于 xqy2006/ModelTrace 的 MIT 授权评分实现改写。
 * 上游源码：https://github.com/xqy2006/ModelTrace
 */
import bank from './unified-bank.json'

const VALUE_MIN = 1
const VALUE_MAX = 355
const DIMENSION = VALUE_MAX - VALUE_MIN + 1
const ALPHA = 0.5

interface ModelTraceModel {
  id: string
  display_name: string
  family?: string
  family_name?: string
  counts: number[]
}

interface ModelTraceBank {
  models: ModelTraceModel[]
  calibration: Record<string, { beta: number, cv_accuracy: number }>
  robust: {
    hellinger: {
      feature_mean: number[]
      feature_scale: number[]
      nuisance_basis: number[][]
      centroids: number[][]
    }
    ordered_blocks?: {
      weight?: number
      feature_mean: number[]
      feature_scale: number[]
      environment_centroids: number[][][]
      nuisance_basis: number[][]
      centroids: number[][]
    }
  }
}

export interface ModelAttributionOutput {
  text: string
  expectedCount: number
}

export interface ModelAttributionResultItem {
  model: string
  displayName: string
  probability: number
  profileSimilarity: number
  family: string
  familyName: string
}

export interface ModelAttributionResult {
  prediction: string
  predictionName: string
  probability: number
  usedOutputs: number
  cvAccuracy: number
  results: ModelAttributionResultItem[]
  diagnostics: Array<{
    parsedNumbers: number
    minimumNumbers: number
    accepted: boolean
  }>
}

function mean(values: number[]) {
  return values.reduce((total, value) => total + value, 0) / values.length
}

function standardize(values: number[]) {
  const center = mean(values)
  const variance = mean(values.map(value => (value - center) ** 2))
  const scale = Math.max(Math.sqrt(variance), 1e-12)
  return values.map(value => (value - center) / scale)
}

function dot(left: number[], right: number[]) {
  let value = 0
  for (let index = 0; index < left.length; index += 1)
    value += (left[index] ?? 0) * (right[index] ?? 0)
  return value
}

function normalized(values: number[]) {
  const scale = Math.max(Math.sqrt(dot(values, values)), 1e-12)
  return values.map(value => value / scale)
}

function subtractBasis(values: number[], basis: number[][]) {
  const output = values.slice()
  for (const vector of basis || []) {
    const projection = dot(output, vector)
    for (let index = 0; index < output.length; index += 1)
      output[index] = (output[index] ?? 0) - projection * (vector[index] ?? 0)
  }
  return output
}

function filledNumbers(length: number, value: number) {
  const values: number[] = []
  for (let index = 0; index < length; index += 1)
    values.push(value)
  return values
}

function parseNumbers(text: string) {
  const runs: number[][] = []
  let current: number[] = []
  let previousEnd = 0
  for (const match of text.matchAll(/\d+/g)) {
    const index = match.index ?? 0
    const separator = text.slice(previousEnd, index)
    const value = Number(match[0])
    if (current.length && /\p{L}/u.test(separator)) {
      runs.push(current)
      current = []
    }
    if (value >= VALUE_MIN && value <= VALUE_MAX)
      current.push(value)
    previousEnd = index + match[0].length
  }
  if (current.length)
    runs.push(current)
  return runs.reduce<number[]>((best, run) => run.length > best.length ? run : best, [])
}

function countNumbers(numbers: number[]) {
  const counts = filledNumbers(DIMENSION, 0)
  for (const number of numbers)
    counts[number - VALUE_MIN] = (counts[number - VALUE_MIN] ?? 0) + 1
  return counts
}

function hellingerFeature(counts: number[]) {
  const total = counts.reduce((sum, value) => sum + value, 0) + ALPHA * DIMENSION
  return counts.map(value => Math.sqrt((value + ALPHA) / total))
}

function splitIntoFour(values: number[]) {
  const base = Math.floor(values.length / 4)
  const remainder = values.length % 4
  const chunks: number[][] = []
  let start = 0
  for (let index = 0; index < 4; index += 1) {
    const size = base + (index < remainder ? 1 : 0)
    chunks.push(values.slice(start, start + size))
    start += size
  }
  return chunks
}

function orderedBlockFeature(numbers: number[]) {
  const pieces: number[] = []
  for (const chunk of splitIntoFour(numbers)) {
    const bins = filledNumbers(16, 0.5)
    for (const value of chunk) {
      const index = Math.min(15, Math.floor(((value - 1) / 355) * 16))
      bins[index] = (bins[index] ?? 0) + 1
    }
    const total = bins.reduce((sum, value) => sum + value, 0)
    pieces.push(...bins.map(value => Math.sqrt(value / total)))
  }
  const lastDigits = filledNumbers(10, 0.5)
  for (const value of numbers)
    lastDigits[value % 10] = (lastDigits[value % 10] ?? 0) + 1
  const total = lastDigits.reduce((sum, value) => sum + value, 0)
  pieces.push(...lastDigits.map(value => Math.sqrt(value / total)))
  return pieces
}

function robustScoreNumbers(numbers: number[], artifact: ModelTraceBank) {
  const counts = countNumbers(numbers)
  const hellinger = artifact.robust.hellinger
  const feature = hellingerFeature(counts)
  let projected = feature.map((value, index) =>
    (value - (hellinger.feature_mean[index] ?? 0)) / (hellinger.feature_scale[index] ?? 1))
  projected = normalized(subtractBasis(projected, hellinger.nuisance_basis))
  const marginal = standardize(hellinger.centroids.map(centroid => dot(projected, centroid)))
  const ordered = artifact.robust.ordered_blocks
  const weight = Number(ordered?.weight || 0)
  if (!ordered || weight === 0)
    return marginal

  const orderedFeature = orderedBlockFeature(numbers)
  const standardized = orderedFeature.map((value, index) =>
    (value - (ordered.feature_mean[index] ?? 0)) / (ordered.feature_scale[index] ?? 1))
  const unit = normalized(standardized)
  const environmentScores = ordered.environment_centroids.map(centroids =>
    centroids.map(centroid => dot(unit, centroid)))
  const template = standardize(ordered.centroids.map((_, modelIndex) =>
    Math.max(...environmentScores.map(scores => scores[modelIndex] ?? Number.NEGATIVE_INFINITY))))
  const nuisanceUnit = normalized(subtractBasis(standardized, ordered.nuisance_basis))
  const nuisance = standardize(ordered.centroids.map(centroid => dot(nuisanceUnit, centroid)))
  const orderedScores = standardize(template.map((value, index) =>
    0.5 * value + 0.5 * (nuisance[index] ?? 0)))
  return marginal.map((value, index) => (1 - weight) * value + weight * (orderedScores[index] ?? 0))
}

function softmax(values: number[]) {
  const maximum = Math.max(...values)
  const weights = values.map(value => Math.exp(value - maximum))
  const total = weights.reduce((sum, value) => sum + value, 0)
  return weights.map(value => value / total)
}

function jsSimilarity(left: number[], right: number[]) {
  const leftTotal = left.reduce((sum, value) => sum + value, 0)
  const rightTotal = right.reduce((sum, value) => sum + value, 0) + ALPHA * DIMENSION
  const p = left.map(value => value / leftTotal)
  const q = right.map(value => (value + ALPHA) / rightTotal)
  const midpoint = p.map((value, index) => (value + (q[index] ?? 0)) / 2)
  const divergence = (values: number[]) => values.reduce((total, value, index) =>
    total + (value ? value * Math.log(value / (midpoint[index] ?? 1)) : 0), 0)
  return 1 - Math.sqrt(((divergence(p) + divergence(q)) / 2) / Math.log(2))
}

export function analyzeModelAttribution(outputs: ModelAttributionOutput[]): ModelAttributionResult {
  const artifact = bank as ModelTraceBank
  const valid: Array<{ counts: number[], scores: number[] }> = []
  const diagnostics = outputs.map((output) => {
    const numbers = parseNumbers(output.text)
    const minimumNumbers = Math.max(80, Math.ceil(output.expectedCount * 0.55))
    const accepted = numbers.length >= minimumNumbers
    if (accepted) {
      valid.push({
        counts: countNumbers(numbers),
        scores: robustScoreNumbers(numbers, artifact),
      })
    }
    return { parsedNumbers: numbers.length, minimumNumbers, accepted }
  })
  if (!valid.length)
    throw new Error('探针回答中的有效数字不足，无法归因')

  const combinedScores = artifact.models.map((_, modelIndex) =>
    mean(valid.map(item => item.scores[modelIndex] ?? 0)))
  const calibrationKey = String(Math.min(valid.length, 3))
  const calibration = artifact.calibration[calibrationKey]
  if (!calibration)
    throw new Error('指纹库缺少当前探针数量的校准参数')
  const probabilities = softmax(combinedScores.map(value => calibration.beta * value))
  const pooledCounts = Array.from({ length: DIMENSION }, (_, index) =>
    valid.reduce((sum, item) => sum + (item.counts[index] ?? 0), 0))
  const results = artifact.models.map((model, index) => ({
    model: model.id,
    displayName: model.display_name,
    probability: probabilities[index] ?? 0,
    profileSimilarity: jsSimilarity(pooledCounts, model.counts),
    family: model.family || 'models',
    familyName: model.family_name || model.family || 'Models',
  })).sort((left, right) => right.probability - left.probability)
  const prediction = results[0]
  if (!prediction)
    throw new Error('指纹库没有候选模型')
  return {
    prediction: prediction.model,
    predictionName: prediction.displayName,
    probability: prediction.probability,
    usedOutputs: valid.length,
    cvAccuracy: calibration.cv_accuracy,
    results,
    diagnostics,
  }
}
