// Fcitx5 addon — the C++ half.
//
// Fcitx5 loads input methods as C++ shared libraries implementing
// fcitx::InputMethodEngine. There is no Rust API and, unlike IBus, no way in
// over D-Bus. So this file is deliberately as thin as it can be: it translates
// Fcitx5's callbacks into calls on the C ABI exported by the Rust crate beside
// it (see src/lib.rs) and does no thinking of its own. Every decision about
// what a key means lives in Rust, where it is unit-tested.

#include <fcitx-utils/i18n.h>
#include <fcitx-utils/key.h>
#include <fcitx/addonfactory.h>
#include <fcitx/addoninstance.h>
// addonfactory.h only forward-declares AddonManager, and the factory below
// calls manager->instance(); without this the class is incomplete at that call.
#include <fcitx/addonmanager.h>
#include <fcitx/candidatelist.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputcontextproperty.h>
#include <fcitx/inputmethodengine.h>
#include <fcitx/inputpanel.h>
#include <fcitx/instance.h>
#include <fcitx/text.h>

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

// ---------------------------------------------------------------------------
// The Rust C ABI. Kept in sync by hand with src/lib.rs.
// ---------------------------------------------------------------------------
extern "C" {
struct XlitState;
XlitState *xlit_new(void);
void xlit_free(XlitState *);
int32_t xlit_process_key(XlitState *, uint32_t keysym, uint32_t modifiers);
void xlit_reset(XlitState *);
const char *xlit_commit(const XlitState *);
const char *xlit_preedit(const XlitState *);
size_t xlit_candidate_count(const XlitState *);
const char *xlit_candidate(const XlitState *, size_t index);
size_t xlit_cursor(const XlitState *);
}

namespace {

/// Modifier bits the Rust side expects. These match the X11 values that both
/// Fcitx5 and IBus use, so the state machine does not have to know which
/// framework it is talking to.
constexpr uint32_t kControl = 1u << 2;
constexpr uint32_t kAlt = 1u << 3;
constexpr uint32_t kRelease = 1u << 30;

uint32_t modifiersOf(const fcitx::KeyEvent &event) {
    uint32_t out = 0;
    const auto states = event.key().states();
    if (states.test(fcitx::KeyState::Ctrl)) {
        out |= kControl;
    }
    if (states.test(fcitx::KeyState::Alt)) {
        out |= kAlt;
    }
    if (event.isRelease()) {
        out |= kRelease;
    }
    return out;
}

/// One Rust handle per input context, owned by Fcitx5's property system so its
/// lifetime follows the context's.
class XlitContext : public fcitx::InputContextProperty {
public:
    XlitContext() : state_(xlit_new()) {}
    ~XlitContext() override { xlit_free(state_); }

    XlitContext(const XlitContext &) = delete;
    XlitContext &operator=(const XlitContext &) = delete;

    XlitState *get() const { return state_; }

private:
    XlitState *state_;
};

} // namespace

class XlitEngine final : public fcitx::InputMethodEngine {
public:
    explicit XlitEngine(fcitx::Instance *instance)
        : instance_(instance),
          factory_([](fcitx::InputContext &) { return new XlitContext(); }) {
        instance_->inputContextManager().registerProperty("xlitState", &factory_);
    }

    void keyEvent(const fcitx::InputMethodEntry &, fcitx::KeyEvent &event) override {
        auto *context = event.inputContext()->propertyFor(&factory_);
        auto *state = context->get();
        if (state == nullptr) {
            return;
        }

        const int handled = xlit_process_key(state, event.key().sym(), modifiersOf(event));

        // Commit first, then redraw: the order the document sees them matters
        // when a key both finishes one word and starts the next.
        const std::string commit = xlit_commit(state);
        if (!commit.empty()) {
            event.inputContext()->commitString(commit);
        }
        updateUI(event.inputContext(), state);

        if (handled != 0) {
            event.filterAndAccept();
        }
    }

    void reset(const fcitx::InputMethodEntry &, fcitx::InputContextEvent &event) override {
        auto *context = event.inputContext()->propertyFor(&factory_);
        if (auto *state = context->get()) {
            // Deliberately does not commit: a half-typed word would otherwise
            // land in whatever the user just clicked on.
            xlit_reset(state);
            updateUI(event.inputContext(), state);
        }
    }

    void deactivate(const fcitx::InputMethodEntry &entry,
                    fcitx::InputContextEvent &event) override {
        reset(entry, event);
    }

private:
    /// Push the preedit and candidate list into the input panel.
    void updateUI(fcitx::InputContext *ic, XlitState *state) {
        auto &panel = ic->inputPanel();
        panel.reset();

        const std::string preedit = xlit_preedit(state);
        if (!preedit.empty()) {
            // The preedit is the raw Latin, underlined — never a conversion.
            // Showing a guess here means text the user never typed moving under
            // their cursor on every keystroke.
            fcitx::Text text;
            text.append(preedit, fcitx::TextFormatFlag::Underline);
            text.setCursor(static_cast<int>(preedit.size()));
            if (ic->capabilityFlags().test(fcitx::CapabilityFlag::Preedit)) {
                panel.setClientPreedit(text);
            } else {
                panel.setPreedit(text);
            }

            const size_t count = xlit_candidate_count(state);
            if (count > 0) {
                auto list = std::make_unique<fcitx::CommonCandidateList>();
                list->setPageSize(9);
                list->setLayoutHint(fcitx::CandidateLayoutHint::Vertical);
                for (size_t i = 0; i < count; ++i) {
                    list->append<fcitx::DisplayOnlyCandidateWord>(
                        fcitx::Text(xlit_candidate(state, i)));
                }
                const size_t cursor = xlit_cursor(state);
                if (cursor < count) {
                    list->setGlobalCursorIndex(static_cast<int>(cursor));
                }
                panel.setCandidateList(std::move(list));
            }
        }

        ic->updatePreedit();
        ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
    }

    fcitx::Instance *instance_;
    fcitx::FactoryFor<XlitContext> factory_;
};

class XlitEngineFactory final : public fcitx::AddonFactory {
public:
    fcitx::AddonInstance *create(fcitx::AddonManager *manager) override {
        return new XlitEngine(manager->instance());
    }
};

FCITX_ADDON_FACTORY(XlitEngineFactory)
