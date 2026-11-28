// Run against the same ImGui objects and font data linked into the localized DLL.
#include <imgui/imgui.h>
#include <imgui/imgui_internal.h>
#include <menu/MenuZh.h>
#include <menu/MenuZhFont.h>
#include <cstdio>
#include <cstring>
#include <string>

int main()
{
    ImGui::CreateContext();
    auto& io = ImGui::GetIO();
    io.IniFilename = nullptr;
    io.BackendFlags |= ImGuiBackendFlags_RendererHasTextures;
    io.DisplaySize = {800, 600};
    io.DeltaTime = 1.0f / 60.0f;
    auto* font = io.Fonts->AddFontDefault();
    ImFontConfig cjk;
    cjk.MergeMode = true;
    cjk.FontDataOwnedByAtlas = false;
    io.Fonts->AddFontFromMemoryTTF(const_cast<unsigned char*>(kMenuFont), sizeof(kMenuFont), 16, &cjk, kMenuGlyphs);
    int width{}, height{};

    bool enabled = false;
    int passes = 2;
    ImGuiID checkboxId = 0;
    ImVec2 click{}, slider{};
    std::string rendered;
    for (int frame = 0; frame < 7; ++frame)
    {
        if (frame == 2 || frame == 4)
        {
            const ImVec2 pos = frame == 2 ? click : slider;
            io.AddMousePosEvent(pos.x, pos.y);
            io.AddMouseButtonEvent(0, true);
        }
        if (frame == 3 || frame == 5)
            io.AddMouseButtonEvent(0, false);
        ImGui::NewFrame();
        if (frame == 0)
            for (auto range = kMenuGlyphs; *range; range += 2)
                for (unsigned cp = range[0]; cp <= range[1]; ++cp)
                    if (!font->GetFontBaked(16)->FindGlyphNoFallback(static_cast<ImWchar>(cp)))
                        return 2;
        ImGui::SetNextWindowPos({0, 0});
        ImGui::SetNextWindowSize({600, 360});
        ImGui::Begin("OptiScaler");
        ImGui::LogToBuffer();
        const char* label = "Enable Neural Rendering##test-nr";
        const auto expected = ImGui::GetID(label);
        ImGui::Checkbox(label, &enabled);
        if (ImGui::GetItemID() != expected || (checkboxId && expected != checkboxId))
            return 3;
        checkboxId = expected;
        click = ImGui::GetItemRectMin();
        click.x += 8;
        click.y += 8;
        ImGui::SetNextItemWidth(180);
        ImGui::SliderInt("Model passes", &passes, 1, 10);
        slider = ImGui::GetItemRectMin();
        slider.x += 155;
        slider.y += 8;
        ImGui::Text("Denoise first: %s", "TEST");
        if (ImGui::CalcTextSize(label, nullptr, true).x != ImGui::CalcTextSize("启用神经渲染").x)
            return 4;
        rendered = ImGui::GetCurrentContext()->LogBuffer.c_str();
        ImGui::LogFinish();
        ImGui::End();
        ImGui::Render();
        auto* atlas = io.Fonts->TexData;
        if (!atlas || !atlas->Pixels || atlas->Width <= 0 || atlas->Height <= 0)
            return 1;
        width = atlas->Width;
        height = atlas->Height;
        atlas->SetTexID(1);
        atlas->SetStatus(ImTextureStatus_OK);
        if (frame && ImGui::GetDrawData()->TotalVtxCount <= 0)
            return 5;
    }
    ImGui::DestroyContext();
    if (!enabled || passes <= 2 || rendered.find("启用神经渲染") == std::string::npos ||
        rendered.find("模型运行次数") == std::string::npos || rendered.find("先降噪：TEST") == std::string::npos)
    {
        std::fprintf(stderr, "checkbox=%d slider=%d labels=%s\n", enabled, passes, rendered.c_str());
        return 6;
    }
    std::printf("PASS: glyph atlas %dx%d, hidden ID stable, checkbox=%d, slider=%d, formatted text rendered\n",
                width, height, enabled, passes);
    return 0;
}
