export const LEVEL_COLORS: Record<string, { bg: string; text: string }> = {
  fatal: { bg: "bg-red-100 dark:bg-red-900/30", text: "text-red-800 dark:text-red-300" },
  error: { bg: "bg-red-50 dark:bg-red-900/20", text: "text-red-700 dark:text-red-400" },
  warning: { bg: "bg-yellow-50 dark:bg-yellow-900/20", text: "text-yellow-700 dark:text-yellow-400" },
  info: { bg: "bg-blue-50 dark:bg-blue-900/20", text: "text-blue-700 dark:text-blue-400" },
  debug: { bg: "bg-gray-50 dark:bg-gray-800/40", text: "text-gray-600 dark:text-gray-400" },
};

export const STATUS_LABELS: Record<string, string> = {
  unresolved: "Unresolved",
  resolved: "Resolved",
  ignored: "Ignored",
};

export const STATUS_COLORS: Record<string, string> = {
  unresolved: "text-orange-600 dark:text-orange-400",
  resolved: "text-green-600 dark:text-green-400",
  ignored: "text-gray-500 dark:text-gray-400",
};
