module.exports = function (api) {
  api.cache(true);
  return {
    // babel-preset-expo 57 registers the react-native-worklets plugin
    // (Reanimated 4) itself; adding it here again applies it twice.
    presets: ["babel-preset-expo"],
  };
};
