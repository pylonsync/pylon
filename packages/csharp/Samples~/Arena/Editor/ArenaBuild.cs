#if UNITY_EDITOR
using System;
using UnityEditor;
using UnityEditor.Build.Reporting;
using UnityEditor.SceneManagement;
using UnityEngine;

namespace Pylon.Samples.Arena
{
    public static class ArenaBuild
    {
        [MenuItem("Pylon/Build Web sample")]
        public static void Web() => Build(BuildTarget.WebGL, "Build/Web");
        [MenuItem("Pylon/Build macOS sample")]
        public static void Mac() => Build(BuildTarget.StandaloneOSX, "Build/PylonArena.app");

        static void Build(BuildTarget target, string output)
        {
            EditorSceneManager.NewScene(NewSceneSetup.DefaultGameObjects, NewSceneMode.Single);
            var sample = new GameObject("Pylon Arena").AddComponent<ArenaSample>();
            sample.serverUrl = Environment.GetEnvironmentVariable("PYLON_SAMPLE_URL") ?? "http://localhost:4321";
            sample.gameObject.AddComponent<WebValidation>();
            var shaderName = UnityEngine.Rendering.GraphicsSettings.currentRenderPipeline != null
                ? "Universal Render Pipeline/Unlit" : "Unlit/Color";
            var material = new Material(Shader.Find(shaderName));
            const string materialPath = "Assets/PylonSample.mat";
            var existing = AssetDatabase.LoadAssetAtPath<Material>(materialPath);
            if (existing != null) { existing.shader = material.shader; UnityEngine.Object.DestroyImmediate(material); material = existing; }
            else AssetDatabase.CreateAsset(material, materialPath);
            sample.sampleMaterial = material;
            EditorSceneManager.SaveScene(EditorSceneManager.GetActiveScene(), "Assets/PylonArena.unity");
            PlayerSettings.WebGL.compressionFormat = WebGLCompressionFormat.Disabled;
            PlayerSettings.runInBackground = true;
            var report = BuildPipeline.BuildPlayer(new BuildPlayerOptions
            {
                scenes = new[] { "Assets/PylonArena.unity" },
                locationPathName = output,
                target = target,
                options = BuildOptions.Development,
            });
            if (report.summary.result != BuildResult.Succeeded) throw new Exception("Pylon sample build failed");
        }
    }
}
#endif
